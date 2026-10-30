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

# cargo audit lists vulnerabilities before warnings, so a log tail can miss the one that
# failed the step. This ends the log with one line per advisory: warnings first, then
# vulnerabilities, then the count, so however many warnings there are, the vulnerability
# lines stay in the tail a failing step prints. A block runs from one "Crate:" line to the
# next; it is a warning when it has a "Warning:" line (unmaintained, unsound, yanked) and a
# vulnerability when it has none.
cargo_audit() {
    local out rc summary vulns reported
    out=$(cargo audit --color never 2>&1)
    rc=$?
    printf '%s\n' "$out"
    summary=$(printf '%s\n' "$out" | awk '
        function flush() {
            if (crate == "") return
            if (kind == "") vl[++v] = sprintf("vulnerability: %s %s %s", id, crate, version)
            else wl[++w] = sprintf("warning: %s %s %s %s", id, crate, version, kind)
            crate = ""
        }
        /^Crate:/ { flush(); crate = $2; version = "none"; id = "none"; kind = "" }
        /^Version:/ { version = $2 }
        /^ID:/ { id = $2 }
        /^Warning:/ { kind = $2 }
        END {
            flush()
            for (i = 1; i <= w; i++) print wl[i]
            for (i = 1; i <= v; i++) print vl[i]
            printf "audit: %d %s, %d %s\n", v, (v == 1 ? "vulnerability" : "vulnerabilities"),
                w, (w == 1 ? "warning" : "warnings")
        }')
    printf '%s\n' "$summary"
    vulns=$(printf '%s\n' "$summary" | grep -c '^vulnerability: ')
    reported=$(printf '%s\n' "$out" | sed -n -E 's/^error: ([0-9]+) vulnerabilit(y|ies) found!$/\1/p')
    if [ "$vulns" -ne "${reported:-0}" ]; then
        echo "audit output could not be parsed: $vulns vulnerability blocks," \
            "cargo audit reports ${reported:-0}"
        return 1
    fi
    [ "$rc" -eq 0 ] || return 1
    [ "$vulns" -eq 0 ]
}

step "cargo fmt --check" cargo fmt --check
step "cargo clippy --all-targets -- -D warnings" cargo clippy --all-targets -- -D warnings
step "cargo test" cargo_test
step_summary '^tests: '
step "cargo build --release --example node" cargo build --release --example node
step "cargo audit" cargo_audit
# The ID pattern keeps cargo audit's own "warning: N allowed warnings found" line out.
step_summary '^(vulnerability: |warning: ([A-Z]+-[0-9A-Za-z-]+|none) |audit: )'
step "dash check (tracked files)" check_tracked_dashes
step "line endings (tracked files)" check_tracked_line_endings
step "banned name (tracked files, commit messages since origin/main)" banned_name
step "crate attributes (src/lib.rs)" check_crate_attributes

if [ "$FAILED" -ne 0 ]; then
    echo "gate: FAILED"
    exit 1
fi
echo "gate: all steps passed"
