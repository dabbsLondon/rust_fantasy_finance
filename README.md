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
docker build -t rust-fantasy-finance:latest .
```

## GitHub Actions

Two workflows are provided:

- **test.yml** – runs `cargo test` on every push.
- **docker.yml** – builds and publishes a versioned image to GitHub Container Registry. The Docker tag uses the package version from `Cargo.toml` with the branch name appended for non-main branches (e.g. `0.1.0` or `0.1.0_feature`). The repository is also tagged with the same value.

The tagging logic lives in a reusable composite action at `.github/actions/version`.


