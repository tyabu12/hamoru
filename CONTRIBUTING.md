# Contributing to hamoru

## Development Setup

```bash
# Ensure Rust stable toolchain is installed
rustup toolchain install stable
rustup component add clippy rustfmt

# Build
cargo build --all-targets

# Run all checks
cargo fmt --all --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo deny check licenses bans sources
```

## Hard Rules

These are non-negotiable. Violations must be caught before commit.

1. **No `unwrap()`** — Use `?` operator or explicit error handling. `expect()` only in test code.
2. **No API keys in code, logs, or commits** — Credentials come from environment variables only.
3. **No provider-specific types outside `provider/`** — All cross-layer communication uses shared types.
4. **No `{previous_output}` in System messages** — Always inject as a separate User Role message.
5. **No new dependencies without confirmation** — Discuss in an ADR first.
6. **Doc comments on public functions** — `#[warn(missing_docs)]` is enforced.
7. **No code without tests** — TDD is mandatory from Phase 1 onward. Phase 0 is exempt (compile + clippy clean).

## Commit Messages

Follow [Conventional Commits](https://www.conventionalcommits.org/): `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`, `ci:`

Prefix the subject line with a single emoji. Keep under 72 characters.

Examples:
- `✨ feat: teach Policy Engine to play favorites`
- `🐛 fix: stop workflows from ghosting mid-step`
- `♻️ refactor: untangle provider spaghetti`

## Testing Policy

- **TDD workflow**: trait → test → impl
- Providers: unit tests with mock trait implementations; integration tests marked `#[ignore]`
- Layers 3-5: unit tests with mock Provider + mock Telemetry
- Coverage target: 80%+

## Architecture Decision Records

Record architectural decisions in `docs/decisions/` as ADRs. Format: `NNN-<short-slug>.md`.

## Code Quality

- Error types unified under `HamoruError` using `thiserror`
- `async` functions return `Result<T, HamoruError>` by default
- `clippy -- -D warnings` enforced in CI
- Formatting follows `cargo fmt`
