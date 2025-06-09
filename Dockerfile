# syntax=docker/dockerfile:1

FROM rust:1 as builder
WORKDIR /usr/src/app
COPY . .
RUN cargo install --path . --locked

FROM debian:bullseye-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /usr/local/cargo/bin/rust_fantasy_finance /usr/local/bin/rust_fantasy_finance
EXPOSE 3000
CMD ["rust_fantasy_finance"]

