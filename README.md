# Webhook Server

A lightweight and flexible webhook server built with Rust and Actix Web for receiving, caching, and broadcasting webhooks via WebSockets.

## Features

- General-purpose webhook receiver
- GitHub webhook support with signature verification
- WebSocket broadcasting of received webhooks
- Recent events cache with REST API access
- Authentication for WebSocket and REST API connections
- Environment variable configuration

## Endpoints

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/webhook` | POST | General-purpose webhook receiver |
| `/github` | POST | GitHub-specific webhook receiver with signature verification |
| `/health` | GET | Health check endpoint |
| `/ws` | GET | WebSocket connection endpoint (requires authentication) |
| `/recent-events` | GET | Retrieve recently received webhooks (requires authentication) |

## Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `PORT` | Port the server listens on | `8080` |
| `CLIENT_SECRET` | Bearer token for WebSocket/API authentication | None (authentication disabled if not set) |
| `WEBHOOK_SECRET` | Secret for GitHub webhook signature verification | None (verification fails if not set) |
| `MAX_CACHED_EVENTS` | Maximum number of events to store in memory | `100` |

## Type Definitions

### TimestampedEvent

```rust
struct TimestampedEvent {
    timestamp: u64,         // Unix timestamp in milliseconds
    payload: Value,         // The webhook payload (JSON)
    event_type: Option<String>,  // Optional event type (for GitHub webhooks)
}
```

## Authentication

### WebSocket and REST API Authentication

Authentication is done using Bearer tokens in the HTTP Authorization header. Set the `CLIENT_SECRET` environment variable to enable authentication.

Example:
```
Authorization: Bearer your-secret-token
```

### GitHub Webhook Signature Verification

GitHub webhooks are verified using the `x-hub-signature-256` header. Set the `WEBHOOK_SECRET` environment variable with the same secret configured in your GitHub webhook settings.

## Usage Examples

### Setting up the server

```bash
# Set environment variables
export PORT=9000
export CLIENT_SECRET=your-client-token
export WEBHOOK_SECRET=your-github-webhook-secret
export MAX_CACHED_EVENTS=200

# Run the server
cargo run
```

### Setting up a GitHub webhook

1. Go to your GitHub repository settings
2. Navigate to Webhooks > Add webhook
3. Set the Payload URL to `https://your-server.com/github`
4. Set the Content type to `application/json`
5. Set the Secret to the same value as your `WEBHOOK_SECRET` environment variable
6. Select the events you want to receive
7. Disable SSL verification

### Connecting to the WebSocket

```javascript
// JavaScript example
const socket = new WebSocket('wss://your-server.com/ws');

// Add authorization header
socket.addEventListener('open', function (event) {
    socket.send('Authorization: Bearer your-client-token');
});

// Listen for messages
socket.addEventListener('message', function (event) {
    const data = JSON.parse(event.data);
    console.log('Received webhook:', data);
});
```

### Retrieving recent events

```bash
curl -H "Authorization: Bearer your-client-token" https://your-server.com/recent-events
```

## Building and Deployment

### Build from source

```bash
cargo build --release
```

### Run with Docker

```bash
docker build -t webhook-server .
docker run -p 8080:8080 \
  -e PORT=8080 \
  -e CLIENT_SECRET=your-client-token \
  -e WEBHOOK_SECRET=your-github-webhook-secret \
  -e MAX_CACHED_EVENTS=100 \
  webhook-server
```

## License

MIT
