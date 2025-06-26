# Instructions for AI Agents

- Always run `cargo test` before committing. Continuous integration runs tests for all targets, ensures a release build, and starts the server for a short check.
- Docker images pushed by GitHub Actions use commit-aware tags computed by `.github/actions/version`.
  Builds on `main` bump the minor version number automatically; other branches
  keep the version from `Cargo.toml` and append the branch name. All tags end
  with the short commit SHA to prevent conflicts.
- The tag calculation and repository tagging are implemented in
  `.github/actions/version`.

 - The service exposes `POST /holdings/transaction`, `GET /holdings/orders`,
   `GET /holdings/orders/<user>`, `GET /holdings`, `GET /holdings/<user>`,
  `GET /market/prices`, `GET /market/symbols`, and `GET /activities/<id>`. Transactions are stored in memory and
  persisted to Parquet files under `data/<user>/orders.parquet`. Market data is refreshed every two
  minutes and daily closes are appended to `data/market/<symbol>/prices.parquet`.
  Tests should avoid relying on network access and use temporary directories when touching
  the filesystem.

Example request for adding a transaction:

```bash
curl -X POST http://localhost:3000/holdings/transaction \
  -H 'content-type: application/json' \
  -d '{"user":"demo","symbol":"XYZ","amount":1,"price":1.23}'
```

See `postman_collection.json` for complete examples.

