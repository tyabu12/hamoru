#!/usr/bin/env bash
# smoke-test.sh — E2E smoke test for the hamoru CLI.
#
# Binary is `hamoru-cli` (from crate package name; no [[bin]] override).
# The `hamoru` name shown in --help comes from #[command(name = "hamoru")]
# and does not affect the binary filename.
#
# Usage:
#   bash scripts/smoke-test.sh              # auto-detect (offline if no API key)
#   bash scripts/smoke-test.sh --offline    # force offline only
#   bash scripts/smoke-test.sh --verbose    # show stdout/stderr for every test
#   HAMORU_BIN=/path/to/hamoru-cli bash scripts/smoke-test.sh  # pre-built binary
#
# Exit codes:
#   0 — all attempted tests passed
#   1 — one or more tests failed
#   2 — configuration error (build failure, missing binary, etc.)

set -euo pipefail

# ---------------------------------------------------------------------------
# Configuration
# ---------------------------------------------------------------------------

VERBOSE=false
OFFLINE=false

for arg in "$@"; do
  case "$arg" in
    --offline)  OFFLINE=true ;;
    --verbose|-v) VERBOSE=true ;;
    *)
      echo "Unknown flag: $arg" >&2
      echo "Usage: bash $0 [--offline] [--verbose|-v]" >&2
      exit 2
      ;;
  esac
done

# Prevent verbose tracing from leaking secrets via reqwest / tracing output.
export RUST_LOG="${RUST_LOG:-warn}"

# Portable timeout (GNU coreutils: timeout, macOS Homebrew: gtimeout)
if command -v timeout > /dev/null 2>&1; then
  TIMEOUT_CMD=timeout
elif command -v gtimeout > /dev/null 2>&1; then
  TIMEOUT_CMD=gtimeout
else
  TIMEOUT_CMD=""
fi

# ---------------------------------------------------------------------------
# Resolve binary
# ---------------------------------------------------------------------------

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN=""

if [[ -n "${HAMORU_BIN:-}" ]]; then
  # User-provided binary — validate
  if [[ ! -f "$HAMORU_BIN" ]]; then
    echo "ERROR: HAMORU_BIN=$HAMORU_BIN does not exist." >&2
    exit 2
  fi
  if [[ ! -x "$HAMORU_BIN" ]]; then
    echo "ERROR: HAMORU_BIN=$HAMORU_BIN is not executable." >&2
    exit 2
  fi
  # Resolve to absolute path
  BIN="$(cd "$(dirname "$HAMORU_BIN")" && pwd)/$(basename "$HAMORU_BIN")"
else
  # Build from source
  echo "Building hamoru-cli..."
  if ! cargo build --manifest-path "${REPO_ROOT}/Cargo.toml" 2>&1; then
    echo "ERROR: cargo build failed. Set HAMORU_BIN to use a pre-built binary." >&2
    exit 2
  fi
  BIN="${REPO_ROOT}/target/debug/hamoru-cli"
  if [[ ! -x "$BIN" ]]; then
    echo "ERROR: Binary not found at $BIN after build." >&2
    exit 2
  fi
fi

echo "# hamoru smoke test"
echo "# Binary: $BIN"

# ---------------------------------------------------------------------------
# Temp directory with cleanup
# ---------------------------------------------------------------------------

# Use WORK_DIR (not TMPDIR) to avoid shadowing the system temp directory variable.
WORK_DIR="$(mktemp -d)"

cleanup() {
  if [[ -n "${WORK_DIR:-}" && -d "$WORK_DIR" ]]; then
    rm -rf "$WORK_DIR"
  fi
}
trap cleanup EXIT

cd "$WORK_DIR"

# ---------------------------------------------------------------------------
# Test harness
# ---------------------------------------------------------------------------

PASSED=0
FAILED=0
SKIPPED=0
TOTAL=0
STDOUT_FILE="$WORK_DIR/.test_stdout"
STDERR_FILE="$WORK_DIR/.test_stderr"

# run_test <name> <expected_exit> <cmd...>
#   expected_exit: a number, or "nonzero" for any non-zero code.
run_test() {
  local name="$1" expected_exit="$2"
  shift 2

  TOTAL=$((TOTAL + 1))

  local actual_exit=0
  "$@" > "$STDOUT_FILE" 2> "$STDERR_FILE" || actual_exit=$?

  local ok=false
  if [[ "$expected_exit" == "nonzero" ]]; then
    [[ "$actual_exit" -ne 0 ]] && ok=true
  else
    [[ "$actual_exit" -eq "$expected_exit" ]] && ok=true
  fi

  if $ok; then
    echo "[PASS] $name"
    PASSED=$((PASSED + 1))
    if $VERBOSE; then
      [[ -s "$STDOUT_FILE" ]] && sed 's/^/  stdout: /' "$STDOUT_FILE"
      [[ -s "$STDERR_FILE" ]] && sed 's/^/  stderr: /' "$STDERR_FILE"
    fi
  else
    echo "[FAIL] $name (expected exit=$expected_exit, got=$actual_exit)"
    FAILED=$((FAILED + 1))
    # Always show output on failure
    [[ -s "$STDOUT_FILE" ]] && sed 's/^/  stdout: /' "$STDOUT_FILE"
    [[ -s "$STDERR_FILE" ]] && sed 's/^/  stderr: /' "$STDERR_FILE"
  fi
}

# assert_exists <path> <description>
assert_exists() {
  local path="$1" desc="$2"
  TOTAL=$((TOTAL + 1))
  if [[ -e "$path" ]]; then
    echo "[PASS] $desc"
    PASSED=$((PASSED + 1))
  else
    echo "[FAIL] $desc (not found: $path)"
    FAILED=$((FAILED + 1))
  fi
}

# assert_contains <file> <pattern> <description>
assert_contains() {
  local file="$1" pattern="$2" desc="$3"
  TOTAL=$((TOTAL + 1))
  if grep -q "$pattern" "$file" 2>/dev/null; then
    echo "[PASS] $desc"
    PASSED=$((PASSED + 1))
  else
    echo "[FAIL] $desc (pattern '$pattern' not found)"
    FAILED=$((FAILED + 1))
    if $VERBOSE && [[ -s "$file" ]]; then
      sed 's/^/  content: /' "$file" | head -20
    fi
  fi
}

skip_test() {
  local desc="$1" reason="$2"
  TOTAL=$((TOTAL + 1))
  SKIPPED=$((SKIPPED + 1))
  echo "[SKIP] $desc ($reason)"
}

# Config backup/restore helpers — used by Ollama tests that swap hamoru.yaml.
backup_config() {
  cp -r "$WORK_DIR/.hamoru" "$WORK_DIR/.hamoru.bak"
}

restore_config() {
  if [[ -d "$WORK_DIR/.hamoru.bak" ]]; then
    rm -rf "$WORK_DIR/.hamoru"
    cp -r "$WORK_DIR/.hamoru.bak" "$WORK_DIR/.hamoru"
  fi
}

# ---------------------------------------------------------------------------
# GROUP 1: Offline tests (no API key required)
# ---------------------------------------------------------------------------

echo ""
echo "# GROUP 1: Offline tests"

run_test "--help exits 0" 0 "$BIN" --help
run_test "--version exits 0" 0 "$BIN" --version

run_test "init creates .hamoru/ directory" 0 "$BIN" init
assert_exists ".hamoru/hamoru.yaml" ".hamoru/hamoru.yaml exists after init"
assert_exists ".hamoru/hamoru.policy.yaml" ".hamoru/hamoru.policy.yaml exists after init"
run_test "init is idempotent" 0 "$BIN" init
backup_config

# providers list requires build_registry() which eagerly resolves API keys.
# Use a fake key since list_models() only reads the hardcoded catalog (no HTTP).
run_test "providers list succeeds" 0 \
  env HAMORU_ANTHROPIC_API_KEY=fake-key "$BIN" providers list

# Reuse $STDOUT_FILE from the run_test above (already captured providers list output).
assert_contains "$STDOUT_FILE" "claude" "providers list output contains 'claude'"

run_test "run (no args) exits non-zero" nonzero "$BIN" run

# Phase 4a: telemetry/metrics/plan commands (no provider needed, init-only)
run_test "plan (empty telemetry)" 0 "$BIN" plan
assert_contains "$STDOUT_FILE" "No telemetry data" "plan output shows no-data message"

run_test "metrics (empty telemetry)" 0 "$BIN" metrics --period 7d
assert_contains "$STDOUT_FILE" "Total requests: 0" "metrics shows zero requests"

run_test "telemetry show (empty store)" 0 "$BIN" telemetry show
assert_contains "$STDOUT_FILE" "Total entries: 0" "telemetry show reports zero entries"

# Phase 4a: workflow file-not-found error (fake key to pass build_registry)
run_test "run -w nonexistent file" nonzero \
  env HAMORU_ANTHROPIC_API_KEY=fake-key "$BIN" run -w nonexistent.yaml "test"
assert_contains "$STDERR_FILE" "Failed to read workflow file" \
  "workflow file-not-found produces actionable error"

# Phase 4a: Ollama-only config (no API key, no server needed — catalog fallback)
cat > "$WORK_DIR/.hamoru/hamoru.yaml" <<'YAML'
version: "1"
providers:
  - name: local
    type: ollama
    endpoint: http://127.0.0.1:11434
YAML
run_test "providers list (ollama-only config)" 0 "$BIN" providers list
assert_contains "$STDOUT_FILE" "llama3.3" "ollama catalog contains llama3.3"
restore_config

# ---------------------------------------------------------------------------
# GROUP 2: Online tests (requires HAMORU_ANTHROPIC_API_KEY)
# ---------------------------------------------------------------------------

echo ""
echo "# GROUP 2: Online tests"

if $OFFLINE; then
  skip_test "providers test" "--offline flag set"
  skip_test "run -p cost-optimized" "--offline flag set"
  skip_test "run -m claude:claude-haiku-4-5" "--offline flag set"
  skip_test "run --tags review" "--offline flag set"
  skip_test "run -w workflow (single step)" "--offline flag set"
elif [[ -z "${HAMORU_ANTHROPIC_API_KEY:-}" ]]; then
  skip_test "providers test" "HAMORU_ANTHROPIC_API_KEY not set"
  skip_test "run -p cost-optimized" "HAMORU_ANTHROPIC_API_KEY not set"
  skip_test "run -m claude:claude-haiku-4-5" "HAMORU_ANTHROPIC_API_KEY not set"
  skip_test "run --tags review" "HAMORU_ANTHROPIC_API_KEY not set"
  skip_test "run -w workflow (single step)" "HAMORU_ANTHROPIC_API_KEY not set"
else
  run_test "providers test" 0 "$BIN" providers test
  run_test "run -p cost-optimized" 0 \
    "$BIN" run -p cost-optimized "Reply with only the word OK" --no-stream
  run_test "run -m claude:claude-haiku-4-5" 0 \
    "$BIN" run -m claude:claude-haiku-4-5 "Reply with only the word OK" --no-stream

  # Phase 4a: tag-based routing (tags: [review] → quality-first → sonnet)
  run_test "run --tags review" 0 \
    "$BIN" run --tags review "Reply with only the word OK" --no-stream

  # Phase 4a: workflow execution (single step, no transitions → implicit COMPLETE)
  # Uses default policy → cost-optimized → claude-haiku-4-5
  cat > "$WORK_DIR/smoke-workflow.yaml" <<'YAML'
name: smoke-test
max_iterations: 2
steps:
  - name: respond
    instruction: "{task}"
YAML
  run_test "run -w workflow (single step)" 0 \
    "$BIN" run -w "$WORK_DIR/smoke-workflow.yaml" "Reply with only the word OK"
fi

# ---------------------------------------------------------------------------
# GROUP 3: Ollama tests (local LLM, no API key required)
# ---------------------------------------------------------------------------

echo ""
echo "# GROUP 3: Ollama tests"

OLLAMA_AVAILABLE=false
if ! $OFFLINE; then
  # 127.0.0.1 bypasses DNS (no rebinding risk). --max-time 2 prevents hangs.
  if curl -sf --max-time 2 http://127.0.0.1:11434/api/tags > /dev/null 2>&1; then
    OLLAMA_AVAILABLE=true
  fi
fi

if $OLLAMA_AVAILABLE; then
  # Swap to Ollama-only config for clean testing
  cat > "$WORK_DIR/.hamoru/hamoru.yaml" <<'YAML'
version: "1"
providers:
  - name: local
    type: ollama
    endpoint: http://127.0.0.1:11434
YAML

  run_test "providers test (ollama)" 0 "$BIN" providers test

  # Pick the smallest (lightest) pulled model for fast inference.
  # jq: sort by disk size (correlates with parameter count). grep: first model as fallback.
  OLLAMA_TAGS=$(curl -sf --max-time 2 http://127.0.0.1:11434/api/tags || true)
  if command -v jq > /dev/null 2>&1; then
    OLLAMA_MODEL=$(printf '%s\n' "$OLLAMA_TAGS" \
      | jq -r '[.models[] | {name, size}] | sort_by(.size) | .[0].name // empty')
  else
    OLLAMA_MODEL=$(printf '%s\n' "$OLLAMA_TAGS" \
      | grep -o '"name":"[^"]*"' | head -1 | cut -d'"' -f4)
  fi

  if [[ -n "$OLLAMA_MODEL" ]]; then
    # Portable timeout prevents hangs on slow inference (stock macOS lacks timeout)
    run_test "run -m local:$OLLAMA_MODEL (ollama)" 0 \
      ${TIMEOUT_CMD:+"$TIMEOUT_CMD" 60} "$BIN" run -m "local:$OLLAMA_MODEL" "Reply with only the word OK" --no-stream
  else
    skip_test "run -m local:<model> (ollama)" "no models available"
  fi

  restore_config
else
  skip_test "providers test (ollama)" "Ollama server not running"
  skip_test "run -m local:<model> (ollama)" "Ollama server not running"
fi

# ---------------------------------------------------------------------------
# Summary
# ---------------------------------------------------------------------------

echo ""
echo "# $PASSED passed, $FAILED failed, $SKIPPED skipped (total: $TOTAL)"

if [[ "$FAILED" -gt 0 ]]; then
  exit 1
fi
exit 0
