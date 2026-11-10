#!/usr/bin/env bash
# Local gate: every step runs even after a failure, one PASS or FAIL line per step,
# non-zero exit if any step failed. Run from anywhere inside the repository.

cd "$(git rev-parse --show-toplevel)" || exit 2
. scripts/checks.sh

# The gate uses sccache only when it works, so its result never depends on sccache being
# installed. An exported RUSTC_WRAPPER, set or empty, overrides any build.rustc-wrapper in
# local cargo config.
if sccache --version >/dev/null 2>&1; then
    export RUSTC_WRAPPER=sccache
    echo "sccache: on"
else
    export RUSTC_WRAPPER=
    echo "sccache: off"
fi

LOG_DIR=$(mktemp -d)
trap 'rm -rf "$LOG_DIR"' EXIT
FAILED=0

STEP_COUNT=0

step() {
    local name=$1
    shift
    STEP_COUNT=$((STEP_COUNT + 1))
    LAST_LOG="$LOG_DIR/$STEP_COUNT.log"
    if "$@" >"$LAST_LOG" 2>&1; then
        echo "PASS $name"
        LAST_PASSED=1
    else
        echo "FAIL $name"
        tail -n 40 "$LAST_LOG" | sed 's/^/    /'
        FAILED=1
        LAST_PASSED=0
    fi
}

# Prints lines matching a pattern from the last step's log. A failing step already
# printed its log tail, so this prints only after a pass and never repeats a line.
step_summary() {
    [ "$LAST_PASSED" -eq 1 ] || return 0
    grep -E "$1" "$LAST_LOG" | sed 's/^/    /'
}

cargo_test() {
    local out rc
    out=$(cargo test 2>&1)
    rc=$?
    printf '%s\n' "$out"
    printf '%s\n' "$out" | check_cargo_test_output "$rc"
}

step "cargo fmt --check" cargo fmt --check
step "cargo clippy --all-targets -- -D warnings" cargo clippy --all-targets -- -D warnings
step "cargo test" cargo_test
step_summary '^tests: '
step "cargo build --release --example node" cargo build --release --example node
step "cargo audit" check_cargo_audit
# The ID pattern keeps cargo audit's own "warning: N allowed warnings found" line out.
step_summary '^(vulnerability: |warning: ([A-Z]+-[0-9A-Za-z-]+|none) |audit: )'
step "dash check (tracked files)" check_tracked_dashes
step "line endings (tracked files)" check_tracked_line_endings
step "banned name (tracked files, commit messages since origin/main)" check_banned_name_since_origin
step "crate attributes (src/lib.rs)" check_crate_attributes
step "git dependencies (Cargo.toml, Cargo.lock)" check_git_dependencies Cargo.toml Cargo.lock
step_summary '^git dependencies: '
step "release age (Cargo.lock vs origin/main)" python3 -I scripts/release_age.py
step_summary '^release age: '
step "gate self-test" scripts/tests/run.sh
step_summary '^self-test: '

if [ "$FAILED" -ne 0 ]; then
    echo "gate: FAILED"
    exit 1
fi
echo "gate: all steps passed"
