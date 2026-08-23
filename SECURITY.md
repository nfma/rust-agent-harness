# Security Policy

## Supported Versions

Security fixes are made on `trunk` and included in the next GitHub release.

## Reporting a Vulnerability

Use GitHub's private security advisory flow for `nfma/rust-agent-harness`. Do not
open a public issue containing tokens, authorization codes, session contents,
account data, or exploit details. Include the affected commit or release, impact,
reachable input, and a minimal reproduction when safe.

## System and Scope

The harness is a local Rust CLI for OpenAI Codex browser authorization, macOS
Keychain credential continuity, bounded model requests, terminal-safe output,
and private local session persistence. OAuth callback handling, refresh
coordination, provider transport, credential storage, session files, rendering,
release artifacts, and CI workflows are in scope.

## Threat Model and Trust Boundaries

Treat callback requests, provider responses, redirects, SSE events, prompts,
model output, stored credential bytes, session files, filesystem paths,
environment values, terminal capabilities, and concurrent local processes as
attacker-controlled or corruptible. The harness-owned Keychain coordinates and
an explicit local invocation are trusted only for the requested operation.

## Security Invariants

- Tokens, authorization codes, PKCE material, account identifiers, prompts and
  model output must not appear in diagnostics beyond the documented local
  session record and terminal output.
- Only the harness-owned Keychain item may be read, replaced, or deleted; failed
  refresh, validation, persistence, or output must not corrupt the prior record.
- OAuth state, redirect origin, callback ports, response sizes, timeouts and
  refresh/replay counts must be exact and bounded. Redirects and malformed
  responses fail closed without contacting an unintended origin.
- Session and lock paths must be private, canonical, symlink-safe and atomic;
  concurrent callers must not double-spend a refresh token or publish partial
  state.
- Untrusted provider text must be rendered without terminal or bidirectional
  control injection, and errors must use fixed redacted categories.
- Release and CI workflows must use immutable actions, least privilege,
  verified dependencies, and protected release inputs.

## Reportable Findings and Severity Context

Report credential disclosure or cross-account access, callback or redirect
confusion, replay beyond the documented single retry, token corruption, unsafe
Keychain or session-file access, traversal or symlink escapes, terminal
injection, unbounded network parsing, and release-integrity bypasses. Findings
reachable during ordinary login, refresh, model use, or session inspection are
higher impact than opt-in test-only behavior.

## Known Limitations

Keychain prompts and provider availability are external and can exceed local
timing expectations. Model-content safety, subscription entitlement, and the
security of the OpenAI service are outside this repository's control, but unsafe
handling of their data by the harness remains in scope.
