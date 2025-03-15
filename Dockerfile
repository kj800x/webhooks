# Build Stage
FROM rust:1.85-alpine AS builder
WORKDIR /usr/src/
RUN apk add pkgconfig openssl-dev libc-dev

# - Install dependencies
RUN USER=root cargo new webhook-server
WORKDIR /usr/src/webhook-server
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release

# - Copy source
COPY src ./src
RUN touch src/main.rs && cargo build --release

# Runtime Stage
FROM alpine:latest AS runtime
WORKDIR /app
RUN apk update \
  && apk add openssl ca-certificates

COPY --from=builder /usr/src/webhook-server/target/release/webhook-server ./webhook-server
USER 1000
CMD ["./webhook-server"]
