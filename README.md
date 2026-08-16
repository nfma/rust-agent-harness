# Rust agent harness

This repository contains a runnable CLI, native OpenAI Codex account login and local auth status on macOS, a one-shot OpenAI Codex question, and a fixed model connection diagnostic. The remaining credential lifecycle, conversations, MCP, tools, skills, and the TUI are intentionally deferred.

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

Inspect whether the stored connection is locally usable without contacting OpenAI:

```sh
cargo run --quiet --locked -p harness-cli -- auth status openai-codex
```

The command reads and validates the harness-owned credential without refreshing or changing it. A usable record prints exactly `OpenAI Codex is connected.`; missing, expired, invalid, unsupported, and unreadable records return the existing redacted credential diagnostic. This is a point-in-time local readiness check, not a provider reachability or subscription entitlement check, and it prints no account metadata.

Test that stored connection with one fixed `gpt-5.5` Responses request:

```sh
cargo run --quiet --locked -p harness-cli -- model test openai-codex
```

Ask one question with the same fixed model and bounded transport:

```sh
cargo run --quiet --locked -p harness-cli -- ask openai-codex \
  "Explain in three bullets when Rust's Arc<Mutex<T>> is appropriate."
```

Both model commands read and validate the harness-owned credential without refreshing or changing it, wait for `response.completed`, and then write terminal-safe buffered assistant text once. Terminal and Unicode bidi controls become visible `\u{...}` escapes; all other Unicode, including other invisible format characters, is preserved byte-for-byte. This is not a general confusable or invisible-text detector. `ask` accepts exactly one non-empty UTF-8 prompt of at most 32 KiB. It does not read stdin or files, accept model selection, retain a session, expose streaming output, or enable tools. Because the prompt is a positional argument, it may be recorded in shell history; use it only for ordinary questions and known-safe snippets.

Running the status command against an existing connected account forms an opt-in local Keychain smoke test. Login followed by either model command forms an opt-in live provider smoke test requiring a browser, a ChatGPT subscription with Codex access, Keychain approval, and one paid or included-plan model call. Default tests use deterministic fakes and an in-memory credential store; they never start live OAuth, contact OpenAI, or access the real Keychain.

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
