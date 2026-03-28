# /hamoru-eval

Run hamoru-specific architecture checks on recent changes.

## Instructions

1. Identify files changed since the last commit (or all files if on initial commit).
2. Run these hamoru-specific checks:

### Layer Boundary Check
- Scan for provider-specific types (`Anthropic*`, `Ollama*`, etc.) imported outside `provider/`.
- Verify all cross-layer communication uses shared types from `provider/types.rs`.

### Provider Isolation Check
- Ensure each provider file only imports from `provider/types.rs`, `provider/mod.rs`, and external crates.
- No provider should import from another provider.

### Error Handling Check
- Search for `unwrap()` in non-test code.
- Verify `HamoruError` variants are used appropriately (not generic catch-all).

### Security Check
- Search for hardcoded strings that look like API keys or tokens.
- Verify provider structs with credential fields have manual `Debug` impl.
- Check that `{previous_output}` is not used in system message contexts.

### Build Check
```bash
cargo clippy --all-targets -- -D warnings
cargo fmt --all --check
cargo test --all-targets
```

## Output

Report each check as PASS/FAIL with details for any failures.
