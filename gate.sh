#!/usr/bin/env bash
# Local gate: every step runs even after a failure, one PASS or FAIL line per step,
# non-zero exit if any step failed. Run from anywhere inside the repository.

cd "$(git rev-parse --show-toplevel)" || exit 2
. scripts/checks.sh

# An empty RUSTC_WRAPPER overrides any build.rustc-wrapper in local cargo config,
# so the gate never depends on sccache or another locally installed wrapper.
export RUSTC_WRAPPER=

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
    else
        echo "FAIL $name"
        tail -n 40 "$LAST_LOG" | sed 's/^/    /'
        FAILED=1
    fi
}

banned_name() {
    check_tracked_banned_name || return 1
    if ! git rev-parse -q --verify origin/main >/dev/null; then
        echo "origin/main not found; cannot check commit messages"
        return 1
    fi
    check_message_banned_name origin/main..HEAD
}

# cargo counts an ignored test as success, so the gate reads every "test result:" line
# and fails when any of them reports an ignored test, or when there are none to read.
cargo_test() {
    local out rc lines passed ignored with_ignored
    out=$(cargo test 2>&1)
    rc=$?
    printf '%s\n' "$out"
    read -r lines passed ignored with_ignored < <(printf '%s\n' "$out" | awk '
        /^test result:/ {
            lines++
            for (i = 1; i < NF; i++) {
                if ($(i + 1) ~ /^passed/) passed += $i
                if ($(i + 1) ~ /^ignored/) { ignored += $i; if ($i > 0) with_ignored++ }
            }
        }
        END { printf "%d %d %d %d\n", lines, passed, ignored, with_ignored }')
    echo "tests: $passed passed, $ignored ignored across $lines test result lines"
    [ "$rc" -eq 0 ] || return 1
    if [ "$lines" -eq 0 ]; then
        echo "no test result lines found in cargo test output"
        return 1
    fi
    if [ "$with_ignored" -gt 0 ]; then
        echo "$with_ignored test result lines report ignored tests; the gate requires 0 ignored"
        return 1
    fi
}

step "cargo fmt --check" cargo fmt --check
step "cargo clippy --all-targets -- -D warnings" cargo clippy --all-targets -- -D warnings
step "cargo test" cargo_test
grep '^tests: ' "$LAST_LOG" | sed 's/^/    /'
step "cargo build --release --example node" cargo build --release --example node
step "cargo audit" cargo audit --color never
grep -E '^(Crate|Version|Warning|Title|ID):' "$LAST_LOG" | sed 's/^/    /'
step "dash check (tracked files)" check_tracked_dashes
step "banned name (tracked files, commit messages since origin/main)" banned_name
step "crate attributes (src/lib.rs)" check_crate_attributes

if [ "$FAILED" -ne 0 ]; then
    echo "gate: FAILED"
    exit 1
fi
echo "gate: all steps passed"
