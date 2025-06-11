# Rust Fantasy Finance

This project is a minimal REST API built with [Axum](https://github.com/tokio-rs/axum). It exposes a simple greeting at `GET /` and a holdings service for recording stock transactions.

### Endpoints

- `POST /holdings/transaction` – add a transaction in JSON with `user`, `symbol`, `amount` and `price`.
- `GET /holdings/orders` – list all recorded transactions.
- `GET /holdings/orders/<user>` – list transactions for a specific user. Returns `404` if the user has no orders stored.
- `GET /market/prices` – current price for each symbol held by any user.
- `GET /market/symbols` – list of all symbols currently tracked.

Transactions are kept in memory and flushed to Parquet files under `data/<user>/orders.parquet`.
Market prices are periodically fetched from Yahoo Finance for all symbols found in those orders and served via `/market/prices`. Closing prices are stored under `data/market/<symbol>/prices.parquet` and refreshed every two minutes.
The list of tracked symbols can be retrieved from `/market/symbols`.

#### Example requests

```bash
curl -X POST http://localhost:3000/holdings/transaction \
  -H 'content-type: application/json' \
  -d '{"user":"alice","symbol":"AAPL","amount":5,"price":10.0}'

curl http://localhost:3000/holdings/orders

curl http://localhost:3000/holdings/orders/alice
```

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

## Downloading and running the published image

Images published by the workflow are available from GitHub Container Registry.
Use the tag reported in the workflow summary to pull and run the image:

```bash
docker pull ghcr.io/<github-user>/rust-fantasy-finance:<tag>
docker run --rm -p 3000:3000 ghcr.io/<github-user>/rust-fantasy-finance:<tag>
```

The published images target the `linux/amd64` platform and run on any x86_64
system with Docker. If you're on an Apple Silicon Mac, add
`--platform linux/amd64` **immediately after** `docker run` and before the image
name:

```bash
docker run --rm -p 3000:3000 --platform linux/amd64 \
  ghcr.io/<github-user>/rust-fantasy-finance:<tag>
```

## GitHub Actions

Two workflows are provided:

- **test.yml** – runs `cargo test` on every push.
- **docker.yml** – builds and publishes a versioned image to GitHub Container Registry. Tags include the short commit SHA to avoid collisions. Main branch builds automatically bump the minor version while other branches keep the version from `Cargo.toml` and append the branch name. Examples: `0.1.1-abc123` for main or `0.1.0_feature-abc123` for a branch.

The tagging logic lives in a reusable composite action at `.github/actions/version`.
The workflow requires `contents: write` permissions so the action can push tags to the repository.


