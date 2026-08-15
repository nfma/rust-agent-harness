# Rust agent harness

This repository contains a runnable CLI and native OpenAI Codex account login on macOS. Model requests, the remaining credential lifecycle, MCP, tools, skills, and the TUI are intentionally deferred.

It requires Rust 1.85 or newer.

Inspect the CLI:

```sh
cargo run -p harness-cli -- --version
```

Connect an OpenAI Codex account on macOS:

```sh
cargo run -p harness-cli -- auth login openai-codex
```

The command opens OpenAI's browser authorization flow and prints the same one-time URL as a fallback. On success, it stores a versioned credential under the harness-owned `rust-agent-harness` / `openai-codex:default` macOS Keychain coordinates. It does not read or modify Codex CLI credentials.

This command is also the opt-in live smoke test: it requires a browser, a ChatGPT subscription with Codex access, and Keychain approval. Default tests use local fake endpoints and an in-memory credential store; they never start live OAuth or access the real Keychain.

Verify the workspace:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
```

## SonarQube Cloud

CI-based analysis uses `sonar-project.properties` for project
`nfma_rust-agent-harness`. Automatic Analysis must be disabled under
**Administration → Analysis Method** in SonarQube Cloud. Add a repository
Actions secret named `SONAR_TOKEN`. Dependabot and fork pull requests, which
cannot read Actions secrets, skip the scan with a warning; a missing token on
any other run fails the job. The workflow runs formatting, Clippy, tests with
LCOV coverage, and waits for the quality gate when the scan runs.

## Releases

Pull requests and pushes to `trunk` run the quality gate and build a macOS archive for Apple Silicon. A manual workflow run assembles the archive and `SHA256SUMS` without publishing by default. After merging a version change to `trunk`, push a matching tag such as `v0.1.0` to publish them as a GitHub Release.

The tag version must match the Cargo package version, and the tagged commit must belong to `trunk`. CI verifies and builds releases with Rust 1.85.0 and checks compatibility with current stable Rust. Release binaries are not currently signed or notarized.

## License

Licensed under the MIT License. See [LICENSE](LICENSE).
