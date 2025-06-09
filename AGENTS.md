# Instructions for AI Agents

- Always run `cargo test` before committing. Continuous integration depends on it.
- Docker images pushed by GitHub Actions use commit-aware tags computed by `.github/actions/version`.
  Builds on `main` bump the minor version number automatically; other branches
  keep the version from `Cargo.toml` and append the branch name. All tags end
  with the short commit SHA to prevent conflicts.
- The tag calculation and repository tagging are implemented in
  `.github/actions/version`.

