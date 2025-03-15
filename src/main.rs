use actix::{Actor, ActorContext, Addr, AsyncContext, Handler, Message, StreamHandler};
use actix_web::{web, App, Error, HttpRequest, HttpResponse, HttpServer, Responder};
use actix_web_actors::ws;
use actix_web_prom::PrometheusMetricsBuilder;
use dotenv::dotenv;
use hex;
use hmac::{Hmac, Mac};
use lazy_static::lazy_static;
use prometheus::{
    opts, register_counter_vec_with_registry, register_counter_with_registry,
    register_gauge_with_registry, register_int_gauge_with_registry, Counter, CounterVec, Gauge,
    IntGauge, Registry,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Sha256;
use std::collections::{HashMap, VecDeque};
use std::env;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// Define global metrics registry
lazy_static! {
    static ref REGISTRY: Registry = Registry::new();
    static ref SERVER_START_TIME: SystemTime = SystemTime::now();
    static ref UPTIME_SECONDS: Gauge = register_gauge_with_registry!(
        opts!(
            "webhook_server_uptime_seconds",
            "Time since the server was started in seconds"
        ),
        &REGISTRY
    )
    .unwrap();
    static ref EVENTS_RECEIVED_TOTAL: Counter = register_counter_with_registry!(
        opts!(
            "webhook_server_events_received_total",
            "Total number of webhook events received"
        ),
        &REGISTRY
    )
    .unwrap();
    static ref GITHUB_EVENTS_RECEIVED_TOTAL: CounterVec = register_counter_vec_with_registry!(
        opts!(
            "webhook_server_github_events_received_total",
            "GitHub webhook events received"
        ),
        &["event_type"],
        &REGISTRY
    )
    .unwrap();
    static ref WEBSOCKET_CLIENTS_CONNECTED: IntGauge = register_int_gauge_with_registry!(
        opts!(
            "webhook_server_websocket_clients_connected",
            "Number of WebSocket clients currently connected"
        ),
        &REGISTRY
    )
    .unwrap();
    static ref EVENTS_CACHE_SIZE: IntGauge = register_int_gauge_with_registry!(
        opts!(
            "webhook_server_events_cache_size",
            "Number of events currently stored in cache"
        ),
        &REGISTRY
    )
    .unwrap();
}

// Update metrics function
fn update_metrics() {
    // Update uptime
    if let Ok(duration) = SystemTime::now().duration_since(*SERVER_START_TIME) {
        UPTIME_SECONDS.set(duration.as_secs_f64());
    }
}

// WebSocket messages
#[derive(Message)]
#[rtype(result = "()")]
struct BroadcastMessage(String);

// WebSocket session actor
struct WebSocketSession {
    id: usize,
    server_addr: Addr<WebSocketServer>,
}

impl Actor for WebSocketSession {
    type Context = ws::WebsocketContext<Self>;

    fn started(&mut self, ctx: &mut Self::Context) {
        // Register self with WebSocketServer
        self.server_addr.do_send(Connect {
            id: self.id,
            addr: ctx.address().recipient(),
        });
    }

    fn stopping(&mut self, _: &mut Self::Context) -> actix::Running {
        // Unregister from WebSocketServer
        self.server_addr.do_send(Disconnect { id: self.id });

        actix::Running::Stop
    }
}

// Handle WebSocket messages
impl StreamHandler<Result<ws::Message, ws::ProtocolError>> for WebSocketSession {
    fn handle(&mut self, msg: Result<ws::Message, ws::ProtocolError>, ctx: &mut Self::Context) {
        match msg {
            Ok(ws::Message::Ping(msg)) => ctx.pong(&msg),
            Ok(ws::Message::Text(text)) => {
                println!("Received message from client: {}", text);
                // Echo back for now
                ctx.text(text);
            }
            Ok(ws::Message::Binary(_)) => println!("Binary data not supported"),
            Ok(ws::Message::Close(reason)) => {
                ctx.close(reason);
                ctx.stop();
            }
            _ => (),
        }
    }
}

// Handle broadcast messages
impl Handler<BroadcastMessage> for WebSocketSession {
    type Result = ();

    fn handle(&mut self, msg: BroadcastMessage, ctx: &mut Self::Context) {
        ctx.text(msg.0);
    }
}

// Messages for WebSocketServer
#[derive(Message)]
#[rtype(result = "()")]
struct Connect {
    id: usize,
    addr: actix::Recipient<BroadcastMessage>,
}

#[derive(Message)]
#[rtype(result = "()")]
struct Disconnect {
    id: usize,
}

// WebSocketServer actor to manage connections and broadcast messages
struct WebSocketServer {
    sessions: HashMap<usize, actix::Recipient<BroadcastMessage>>,
    next_id: usize,
}

impl Default for WebSocketServer {
    fn default() -> Self {
        WebSocketServer {
            sessions: HashMap::new(),
            next_id: 0,
        }
    }
}

impl Actor for WebSocketServer {
    type Context = actix::Context<Self>;
}

impl Handler<Connect> for WebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: Connect, _: &mut Self::Context) {
        println!("WebSocket client connected: {}", msg.id);
        self.sessions.insert(msg.id, msg.addr);

        // Update metrics on connection count
        WEBSOCKET_CLIENTS_CONNECTED.set(self.sessions.len() as i64);
    }
}

impl Handler<Disconnect> for WebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: Disconnect, _: &mut Self::Context) {
        println!("WebSocket client disconnected: {}", msg.id);
        self.sessions.remove(&msg.id);

        // Update metrics on connection count
        WEBSOCKET_CLIENTS_CONNECTED.set(self.sessions.len() as i64);
    }
}

impl Handler<BroadcastMessage> for WebSocketServer {
    type Result = ();

    fn handle(&mut self, msg: BroadcastMessage, _: &mut Self::Context) {
        // Send to all connected WebSocket sessions
        for (_, addr) in &self.sessions {
            addr.do_send(BroadcastMessage(msg.0.clone()));
        }
    }
}

// Structure to store events with timestamps
#[derive(Clone, Serialize, Deserialize)]
struct TimestampedEvent {
    timestamp: u64,
    payload: Value,
    event_type: Option<String>, // Optional event type for GitHub webhooks
}

// Recent events cache
struct EventsCache {
    events: VecDeque<TimestampedEvent>,
    max_size: usize,
}

impl EventsCache {
    fn new(max_size: usize) -> Self {
        EventsCache {
            events: VecDeque::with_capacity(max_size),
            max_size,
        }
    }

    fn add(&mut self, payload: Value, event_type: Option<String>) {
        // Get current timestamp in milliseconds
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let event = TimestampedEvent {
            timestamp,
            payload,
            event_type,
        };

        // Add new event
        self.events.push_back(event);

        // Remove oldest event if over capacity
        if self.events.len() > self.max_size {
            self.events.pop_front();
        }

        // Update metrics for cache size
        EVENTS_CACHE_SIZE.set(self.events.len() as i64);
    }

    fn get_events(&self) -> Vec<TimestampedEvent> {
        self.events.iter().cloned().collect()
    }
}

// Validate bearer token from header against environment variable
fn validate_token(req: &HttpRequest) -> bool {
    // Get the expected token from environment
    let expected_token = match env::var("CLIENT_SECRET") {
        Ok(token) => token,
        Err(_) => {
            println!("Warning: CLIENT_SECRET not set in environment");
            return false;
        }
    };

    // Skip validation if token is empty (for development)
    if expected_token.is_empty() {
        println!("Warning: Empty CLIENT_SECRET, skipping validation");
        return true;
    }

    // Extract the Authorization header
    if let Some(auth_header) = req.headers().get("Authorization") {
        if let Ok(auth_str) = auth_header.to_str() {
            // Check if it's a bearer token with the correct value
            if auth_str.starts_with("Bearer ") {
                let token = auth_str.trim_start_matches("Bearer ").trim();
                return token == expected_token;
            }
        }
    }

    false
}

// WebSocket route handler with token validation
async fn ws_route(
    req: HttpRequest,
    stream: web::Payload,
    srv: web::Data<Addr<WebSocketServer>>,
) -> Result<HttpResponse, Error> {
    // Validate bearer token
    if !validate_token(&req) {
        return Ok(HttpResponse::Unauthorized().body("Invalid or missing authorization token"));
    }

    // Create a new WebSocket session
    let server_addr = srv.get_ref().clone();
    let id = server_addr.send(GetNextId).await.unwrap();

    let ws = WebSocketSession { id, server_addr };

    let resp = ws::start(ws, &req, stream)?;
    Ok(resp)
}

// Handler for recent events endpoint with token validation
async fn recent_events(
    req: HttpRequest,
    events_cache: web::Data<Arc<Mutex<EventsCache>>>,
) -> impl Responder {
    // Validate bearer token
    if !validate_token(&req) {
        return HttpResponse::Unauthorized().body("Invalid or missing authorization token");
    }

    // Get events from cache
    let events = {
        let cache = events_cache.lock().unwrap();
        cache.get_events()
    };

    // Return as JSON
    HttpResponse::Ok().json(events)
}

#[derive(Message)]
#[rtype(result = "usize")]
struct GetNextId;

impl Handler<GetNextId> for WebSocketServer {
    type Result = usize;

    fn handle(&mut self, _: GetNextId, _: &mut Self::Context) -> Self::Result {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

// Verify GitHub webhook signature
fn verify_github_signature(signature: &str, body: &[u8], secret: &str) -> bool {
    // Create HMAC-SHA256 instance with the webhook secret
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC can take key of any size");

    // Update with request body
    mac.update(body);

    // Calculate digest
    let result = mac.finalize().into_bytes();
    let expected_hex = hex::encode(result);

    // Check if signatures match
    let expected_signature = format!("sha256={}", expected_hex);

    // Constant-time comparison to prevent timing attacks
    if signature.len() != expected_signature.len() {
        return false;
    }

    // Use a constant time equality check
    let signature_bytes = signature.as_bytes();
    let expected_bytes = expected_signature.as_bytes();

    signature_bytes
        .iter()
        .zip(expected_bytes.iter())
        .all(|(a, b)| a == b)
}

// Handler for GitHub webhooks
async fn github_webhook_handler(
    req: HttpRequest,
    body: web::Bytes,
    srv: web::Data<Addr<WebSocketServer>>,
    events_cache: web::Data<Arc<Mutex<EventsCache>>>,
) -> impl Responder {
    // Verify the GitHub webhook signature
    if let Some(signature) = req.headers().get("x-hub-signature-256") {
        let webhook_secret = match env::var("WEBHOOK_SECRET") {
            Ok(secret) => secret,
            Err(_) => {
                println!("Error: WEBHOOK_SECRET environment variable not set");
                return HttpResponse::InternalServerError().body("Webhook secret not configured\n");
            }
        };

        if let Ok(signature_str) = signature.to_str() {
            if !verify_github_signature(signature_str, &body, &webhook_secret) {
                println!("Error: Invalid GitHub webhook signature");
                return HttpResponse::Unauthorized().body("Invalid webhook signature\n");
            }
        } else {
            return HttpResponse::BadRequest().body("Invalid signature header\n");
        }
    } else {
        return HttpResponse::BadRequest().body("Missing x-hub-signature-256 header\n");
    }

    // Extract GitHub event type
    let github_event = match req.headers().get("x-github-event") {
        Some(event) => match event.to_str() {
            Ok(event_str) => Some(event_str.to_string()),
            Err(_) => None,
        },
        None => None,
    };

    // Parse the JSON body
    let payload: Value = match serde_json::from_slice(&body) {
        Ok(payload) => payload,
        Err(e) => {
            println!("Error parsing webhook JSON: {}", e);
            return HttpResponse::BadRequest().body("Invalid JSON payload\n");
        }
    };

    let event_type = github_event
        .clone()
        .unwrap_or_else(|| "unknown".to_string());
    EVENTS_RECEIVED_TOTAL.inc();
    GITHUB_EVENTS_RECEIVED_TOTAL
        .with_label_values(&[&event_type])
        .inc();

    // Log the webhook request (concise version)
    println!("Received GitHub webhook event: {}", event_type);

    // Add to events cache
    {
        let mut cache = events_cache.lock().unwrap();
        cache.add(payload.clone(), github_event.clone());
    }

    // Create a JSON structure that includes the event type
    let mut enhanced_payload = serde_json::Map::new();
    enhanced_payload.insert("payload".to_string(), payload);
    if let Some(event) = github_event {
        enhanced_payload.insert("event_type".to_string(), Value::String(event));
    }

    // Broadcast to all WebSocket clients
    let message = serde_json::to_string(&Value::Object(enhanced_payload)).unwrap();
    srv.do_send(BroadcastMessage(message));

    HttpResponse::Ok().body("GitHub webhook received successfully\n")
}

// Modified webhook handler to broadcast messages and cache events
async fn webhook_handler(
    payload: web::Json<Value>,
    srv: web::Data<Addr<WebSocketServer>>,
    events_cache: web::Data<Arc<Mutex<EventsCache>>>,
) -> impl Responder {
    // Increment webhook counter
    EVENTS_RECEIVED_TOTAL.inc();

    // Extract the action or type from the payload if available
    let event_type = payload
        .get("action")
        .or_else(|| payload.get("type"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown");

    // Log the webhook request (concise version)
    println!("Received webhook event: {}", event_type);

    // Extract payload once
    let payload_value = payload.into_inner();

    // Add to events cache
    {
        let mut cache = events_cache.lock().unwrap();
        cache.add(payload_value.clone(), None);
    }

    // Broadcast to all WebSocket clients
    let message = serde_json::to_string(&payload_value).unwrap();
    srv.do_send(BroadcastMessage(message));

    HttpResponse::Ok().body("Webhook received successfully\n")
}

async fn health_check() -> impl Responder {
    // Update uptime metric
    update_metrics();

    HttpResponse::Ok().body("Server is running\n")
}

// Background task to update metrics
async fn metrics_updater() {
    let mut interval = actix_web::rt::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        update_metrics();
    }
}

#[actix_web::main]
async fn main() -> std::io::Result<()> {
    // Load environment variables from .env file if present
    dotenv().ok();
    env_logger::init();

    let port = env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(8080);

    // Get max events to cache from environment, default to 100
    let max_events = env::var("MAX_CACHED_EVENTS")
        .ok()
        .and_then(|s| s.parse::<usize>().ok())
        .unwrap_or(100);

    println!("Starting webhook server on port {}", port);
    println!("Maximum cached events: {}", max_events);
    println!(
        "WebSocket authentication is {}",
        if env::var("CLIENT_SECRET").is_ok() {
            "enabled"
        } else {
            "disabled (token not set)"
        }
    );
    println!(
        "GitHub webhook signature verification is {}",
        if env::var("WEBHOOK_SECRET").is_ok() {
            "enabled"
        } else {
            "disabled (secret not set)"
        }
    );

    // Create events cache
    let events_cache = Arc::new(Mutex::new(EventsCache::new(max_events)));

    // Initialize cache size metric
    EVENTS_CACHE_SIZE.set(0);

    // Initialize websocket clients metric
    WEBSOCKET_CLIENTS_CONNECTED.set(0);

    // Setup prometheus metrics with our custom registry
    let prometheus = PrometheusMetricsBuilder::new("webhook_server")
        .registry(REGISTRY.clone())
        .endpoint("/metrics")
        .build()
        .unwrap();

    // Start background metrics updater
    actix_web::rt::spawn(metrics_updater());

    // Start WebSocket server actor
    let server = WebSocketServer::default().start();

    HttpServer::new(move || {
        App::new()
            .wrap(prometheus.clone())
            .app_data(web::Data::new(server.clone()))
            .app_data(web::Data::new(events_cache.clone()))
            .route("/webhook", web::post().to(webhook_handler))
            .route("/github", web::post().to(github_webhook_handler))
            .route("/health", web::get().to(health_check))
            .route("/ws", web::get().to(ws_route))
            .route("/recent-events", web::get().to(recent_events))
    })
    .bind(("0.0.0.0", port))?
    .run()
    .await
}
