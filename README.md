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
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

## SonarQube Cloud

CI-based analysis uses `sonar-project.properties` for project
`nfma_rust-agent-harness`. Automatic Analysis must be disabled under
**Administration → Analysis Method** in SonarQube Cloud. Add `SONAR_TOKEN` as
both an Actions secret and a Dependabot secret; the workflow runs formatting,
Clippy, tests with LCOV coverage, and waits for the quality gate.

## Releases

Pull requests and pushes to `trunk` run the quality gate and build a macOS archive for Apple Silicon. A manual workflow run assembles the archive and `SHA256SUMS` without publishing by default. After merging a version change to `trunk`, push a matching tag such as `v0.1.0` to publish them as a GitHub Release.

The tag version must match the Cargo package version, and the tagged commit must belong to `trunk`. CI verifies and builds releases with Rust 1.85.0 and checks compatibility with current stable Rust. Release binaries are not currently signed or notarized.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
