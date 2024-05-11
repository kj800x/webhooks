# Webhooks

GitHub webhook pubsub server, basically a reverse proxy with extra steps.

# API

## Types

```ts
interface WebhookEvent {
  githubEvent: string;
  timestamp: number; // epoch millis
  payload: any;
}
```

## `POST /webhook`

Receives GitHub webhooks from GitHub.

Requires requests to be properly signed with `WEBHOOK_SECRET`.

## `GET /recent-events`

Return the current cache of events as `WebhookEvent[]`. This is based on
`RETENTION_PERIOD_IN_MINUTES` with [one caveat](#gotchas).

Expects requests to have bearer auth with `CLIENT_SECRET`.

## `WEBSOCKET /`

Subscribe to any new events, as they come in. Messages are `WebhookEvent`. Server will ignore any
messages from the client.

Expects connection requests to have bearer auth with `CLIENT_SECRET`.

# Env Vars

- `PORT` - required - what port to listen on
- `CLIENT_SECRET` - required - what bearer token is required for clients to query or subscribe to
  webhook events
- `WEBHOOK_SECRET` - required - authenticate that a webhook event is actually coming from github
- `RETENTION_PERIOD_IN_MINUTES` - optional - how long to hold onto an event before letting it get
  garbage collected

# Gotchas

The linked list is only pruned of old events when receiving a new event. It's possible to get events
older than `RETENTION_PERIOD_IN_MINUTES` when using the `/recent-events` endpoint if there haven't
been any new events recently.
