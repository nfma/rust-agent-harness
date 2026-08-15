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

## Releases

Pull requests and pushes to `trunk` run the quality gate and build macOS archives for Apple Silicon and Intel. After merging a version change to `trunk`, push a matching tag such as `v0.1.0` to publish both archives and `SHA256SUMS` as a GitHub Release.

The tag version must match the Cargo package version, and the tagged commit must belong to `trunk`. Release binaries are not currently signed or notarized.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
