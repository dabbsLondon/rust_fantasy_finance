# Rust Fantasy Finance

This project is a minimal REST API built with [Axum](https://github.com/tokio-rs/axum). It exposes a single `GET /` endpoint that returns "Hello, world!".

## Running locally

```bash
cargo run
```

The server listens on port `3000`.

## Testing

```bash
cargo test
```

## Building the Docker image

```bash
docker build -t rust_fantasy_finance:latest .
```

## GitHub Actions

Two workflows are provided:

- **test.yml** – runs `cargo test` on every push.
- **docker.yml** – builds and publishes a versioned image to GitHub Container Registry. The image is tagged as `ghcr.io/<owner>/<repo>:<git short sha>`.


