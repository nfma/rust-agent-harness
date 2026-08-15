# Rust agent harness

This repository currently contains the runnable CLI skeleton only. Authentication, providers, persistence, MCP, tools, skills, and the TUI are intentionally deferred.

It requires Rust 1.85 or newer.

Run the demo:

```sh
cargo run -p harness-cli -- --version
```

Verify the skeleton:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
