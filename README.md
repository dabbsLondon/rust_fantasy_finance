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


