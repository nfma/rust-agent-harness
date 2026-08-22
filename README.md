# Rust agent harness

This repository contains a runnable CLI, native OpenAI Codex account login and credential continuity on macOS, local auth status and logout, a retained one-shot OpenAI Codex question, and a fixed model connection diagnostic. Multi-turn conversations, MCP, tools, skills, and the TUI are intentionally deferred.

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

Disconnect the harness from OpenAI Codex locally:

```sh
cargo run --quiet --locked -p harness-cli -- auth logout openai-codex
```

The command immediately deletes only the harness-owned `rust-agent-harness` / `openai-codex:default` Keychain item without reading it. It is idempotent: both a deleted and an already-absent item print exactly `OpenAI Codex is disconnected.` and exit successfully. This is local logout only; it does not contact OpenAI, revoke tokens held elsewhere, or end a ChatGPT session. Reconnect with the existing browser login command.

Test that stored connection with one fixed `gpt-5.5` Responses request:

```sh
cargo run --quiet --locked -p harness-cli -- model test openai-codex
```

Ask one question with the same fixed model and bounded transport:

```sh
cargo run --quiet --locked -p harness-cli -- ask openai-codex \
  "Explain in three bullets when Rust's Arc<Mutex<T>> is appropriate."
```

Every valid `ask` creates a private versioned JSONL session before contacting the model and prints its 32-character identifier to stderr as `Session: <id>`. Successful and failed asks are retained locally by default. A session contains the prompt, completed answer or fixed sanitized failure, creation and entry timestamps, and the initial working directory when it is available and valid Unicode.

Inspect a known session without contacting OpenAI or reading the Keychain:

```sh
cargo run --quiet --locked -p harness-cli -- session show <session-id>
```

Session files remain until you manually remove them. There is no opt-out, list, delete, retention, or automatic-cleanup command in this increment, and the stderr `Session:` line is the only CLI handle. If that line is discarded or lost, the session remains on disk but cannot be reached through the CLI. Files live under:

- macOS: `~/Library/Application Support/rust-agent-harness/sessions/`
- Linux with `XDG_DATA_HOME`: `$XDG_DATA_HOME/rust-agent-harness/sessions/`
- Linux otherwise: `~/.local/share/rust-agent-harness/sessions/`

Both model commands transparently refresh a harness-owned credential when its expiry is unknown, elapsed, or within five minutes. A successful rotation is validated and saved to Keychain before the model request uses it. The first model HTTP `401` may cause one serialized refresh or adoption of another harness process's replacement and one replay; other model failures are not replayed. If refresh is permanently rejected, reconnect with the browser command:

```sh
cargo run --quiet --locked -p harness-cli -- auth login openai-codex
```

`auth status` remains strictly local and read-only: it never refreshes, writes credentials, or contacts OpenAI. It can therefore report an expired record even when the next model command could refresh it successfully.

Refresh coordination waits at most 65 seconds for another harness process. That timeout is a responsiveness cap: it does not cancel the process holding the lock, which may still be waiting for macOS Keychain interaction. Refresh and one-`401` replay paths can take materially longer than a normal one-attempt call. Excluding unbounded Keychain prompts, the combined documented transport and lock budgets can approach 430 seconds.

Model commands wait for `response.completed` and then write terminal-safe buffered assistant text once. Terminal and Unicode bidi controls become visible `\u{...}` escapes; all other Unicode, including other invisible format characters, is preserved byte-for-byte. `session show` applies the same renderer to retained prompt and answer text. This is not a general confusable or invisible-text detector. `ask` accepts exactly one non-empty UTF-8 prompt of at most 32 KiB. It does not read stdin or files, accept model selection, expose streaming output, or enable tools. Because the prompt is both a positional argument and retained local session content, use it only for ordinary questions and known-safe snippets.

Running the status command against an existing connected account forms an opt-in local Keychain smoke test. A logout smoke temporarily destroys the harness-owned connection and must include browser login and a final successful status check to restore and verify access. Login followed by either model command forms an opt-in live provider smoke test requiring a browser, a ChatGPT subscription with Codex access, Keychain approval, and one paid or included-plan model call. Default tests use deterministic fake stores and endpoints plus temporary lock paths; they never start live OAuth, contact OpenAI, or access the real Keychain.

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
