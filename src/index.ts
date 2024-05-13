import express from 'express';
import * as crypto from 'crypto';
import bodyParser from 'body-parser';
import { createServer } from 'http';
import { WebSocketServer } from 'ws';

interface D

// MARK: - Types

interface WebhookEvent {
  githubEvent: string;
  timestamp: number; // epoch millis
  payload: any;
}

interface ListOfEventElem {
  car: WebhookEvent;
  cdr: ListOfEvents;
}
type ListOfEvents = ListOfEventElem | null;

// MARK: - EventsTracker

class EventsTracker {
  // An ordered linked list of events
  private events: ListOfEvents = null;
  private retentionPeriodInMs: number;

  constructor(retentionPeriodInMs: number) {
    this.retentionPeriodInMs = retentionPeriodInMs;
  }

  // Remove any events older than `retentionPeriodInMs` from the start of the linked list
  // Using the fact that the list is ordered by timestamp
  private purgeOldEvents() {
    const oldestAllowed = Date.now() - this.retentionPeriodInMs;
    while (this.events !== null && this.events.car.timestamp < oldestAllowed) {
      this.events = this.events.cdr;
    }
  }

  private sendToSubscribers(event: WebhookEvent) {
    wss.clients.forEach(client => {
      if (client.readyState === 1) {
        client.send(JSON.stringify(event));
      }
    });
  }

  addEvent(githubEvent: string, payload: any) {
    this.purgeOldEvents();

    this.events = {
      car: {
        githubEvent,
        timestamp: Date.now(),
        payload,
      },
      cdr: this.events,
    };

    this.sendToSubscribers(this.events.car);
  }

  [Symbol.iterator]() {
    let listPointer = this.events;

    return {
      next: () => {
        if (listPointer === null) {
          return { done: true, value: null };
        }

        const value = listPointer.car;
        listPointer = listPointer.cdr;
        return { done: false, value };
      },
    };
  }
}

// MARK: - Env Var Helpers

function getEnvVar(name: string): string {
  const value = process.env[name];

  if (!value) {
    throw new Error(`Please provide a ${name}`);
  }

  return value;
}

function getNumericalEnvVar(name: string): number {
  const value = getEnvVar(name);
  const n = parseInt(value, 10);

  if (isNaN(n)) {
    throw new Error(`Please provide a valid ${name}`);
  }

  return n;
}

function getNumericalEnvVarWithDefault(name: string, defaultValue: number): number {
  const value = process.env[name];
  if (!value) {
    return defaultValue;
  }

  const n = parseInt(value, 10);

  if (isNaN(n)) {
    throw new Error(`Please provide a valid ${name}`);
  }

  return n;
}

// MARK: - Vars

const WEBHOOK_SECRET: string = getEnvVar('WEBHOOK_SECRET');
const CLIENT_SECRET: string = getEnvVar('CLIENT_SECRET');
const PORT: number = getNumericalEnvVar('PORT');
const RETENTION_PERIOD_IN_MINUTES: number = getNumericalEnvVarWithDefault(
  'RETENTION_PERIOD_IN_MINUTES',
  60
);
const eventTracker: EventsTracker = new EventsTracker(RETENTION_PERIOD_IN_MINUTES * 1000);
const app = express();
const server = createServer();
const wss = new WebSocketServer({ noServer: true });
server.on('request', app);
server.listen(PORT, () => {
  console.log(`Server is running on port ${PORT}`);
});

// MARK: - Signing

if (!WEBHOOK_SECRET) {
  console.error('Please provide a WEBHOOK_SECRET');
  process.exit(1);
}

if (!CLIENT_SECRET) {
  console.error('Please provide a CLIENT_SECRET');
  process.exit(1);
}

function verifySignature(actual: string, body: string, secret: string) {
  const expected = crypto
    .createHmac('sha256', secret)
    .update(body)
    .digest('hex');
  let trusted = Buffer.from(`sha256=${expected}`, 'ascii');
  let untrusted = Buffer.from(actual, 'ascii');
  return crypto.timingSafeEqual(trusted, untrusted);
}

// MARK: - Webhook handler

app.post('/webhook', bodyParser.text({ type: '*/*' }), (request, response) => {
  const signature = request.headers['x-hub-signature-256'];

  if (typeof signature !== 'string' || !signature) {
    console.log('Webhook without a signature');
    response.status(400).send('Webhook without a signature');
    return;
  }

  if (!verifySignature(signature, request.body, WEBHOOK_SECRET)) {
    console.log('Webhook with invalid signature');
    response.status(400).send('Webhook with invalid signature');
    return;
  }

  const githubEvent = request.headers['x-github-event'];

  if (typeof githubEvent !== 'string' || !githubEvent) {
    console.log('Received a webhook without an event header, or multiple event headers');
    response
      .status(400)
      .send('Received a webhook without an event header, or multiple event headers');
    return;
  }

  response.status(202).send('Accepted');
  console.log('Accepted webhook for event: ' + githubEvent);
  eventTracker.addEvent(githubEvent, JSON.parse(request.body));
});

// MARK: - recent-events handler

app.get('/recent-events', (request, response) => {
  if (request.headers['authorization'] !== `Bearer ${CLIENT_SECRET}`) {
    response.status(401).send('auth bad');
    return;
  }

  const eventsArray = [...eventTracker];
  response.json(eventsArray);
});

// MARK: - websockets

function onSocketError(err: Error) {
  console.error(err);
}

function authenticate(request: any, callback: any) {
  if (request.headers.authorization !== `Bearer ${CLIENT_SECRET}`) {
    callback(new Error('Invalid authorization'), false);
  }

  // This function is not defined on purpose. Implement it with your own logic.
  callback(null, true);
}

server.on('upgrade', function upgrade(request, socket, head) {
  socket.on('error', onSocketError);
  socket.on('close', () => {
    console.log('websocket client disconnected');
  });

  // This function is not defined on purpose. Implement it with your own logic.
  authenticate(request, function next(err: null | Error, client: any) {
    if (err || !client) {
      socket.write('HTTP/1.1 401 Unauthorized\r\n\r\n');
      socket.destroy();
      return;
    }

    socket.removeListener('error', onSocketError);

    wss.handleUpgrade(request, socket, head, function done(ws) {
      wss.emit('connection', ws, request, client);
      console.log('websocket client connected');
    });
  });
});
