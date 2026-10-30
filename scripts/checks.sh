#!/usr/bin/env bash
# Shared checks for gate.sh and the hooks in .githooks/.
# Banned characters and names are built from escapes or bracket patterns so this
# file never contains them literally and cannot match itself.

EM_DASH=$'\xe2\x80\x94'
EN_DASH=$'\xe2\x80\x93'
CR=$'\r'
BANNED_NAME_RE='c[l]aude'
LOCAL_ONLY_PATH_RE='(^|/)c[l]aude\.md$|^\.c[l]aude/|^private/|(^|/)\.cargo/config(\.toml)?$'
COMMIT_SUBJECT_RE='^(feat|fix|docs|test|refactor|chore)\([a-z0-9_./-]+\): [^ ].*$'

check_tracked_dashes() {
    if git grep -n -I -F -e "$EM_DASH" -e "$EN_DASH" -- .; then
        echo "em or en dash found in tracked files"
        return 1
    fi
}

check_tracked_line_endings() {
    if git grep -n -I -F -e "$CR" -- .; then
        echo "CR found in tracked files"
        return 1
    fi
}

check_tracked_banned_name() {
    if git grep -n -I -i -E "$BANNED_NAME_RE" -- .; then
        echo "banned name found in tracked files"
        return 1
    fi
}

check_message_banned_name() {
    local range=$1 hits
    hits=$(git log --format='%h %B' "$range" | grep -i -E "$BANNED_NAME_RE")
    if [ -n "$hits" ]; then
        echo "$hits"
        echo "banned name found in commit messages in $range"
        return 1
    fi
}

check_crate_attributes() {
    local missing=0 attr
    for attr in '#![forbid(unsafe_code)]' '#![deny(warnings)]'; do
        if ! grep -q -x -F "$attr" src/lib.rs; then
            echo "src/lib.rs is missing $attr"
            missing=1
        fi
    done
    return $missing
}

check_staged_paths() {
    local hits
    hits=$(git diff --cached --name-only --diff-filter=ACMR | grep -i -E "$LOCAL_ONLY_PATH_RE")
    if [ -n "$hits" ]; then
        echo "$hits"
        echo "local-only paths are staged; unstage them"
        return 1
    fi
}

staged_added_lines() {
    git diff --cached -U0 --no-color --no-ext-diff | grep '^+' | grep -v '^+++ '
}

check_staged_dashes() {
    if staged_added_lines | grep -F -e "$EM_DASH" -e "$EN_DASH"; then
        echo "em or en dash in staged added lines"
        return 1
    fi
}

check_staged_banned_name() {
    local rc=0
    if git diff --cached --name-only --diff-filter=ACMR | grep -i -E "$BANNED_NAME_RE"; then
        echo "banned name in staged paths"
        rc=1
    fi
    if staged_added_lines | grep -i -E "$BANNED_NAME_RE"; then
        echo "banned name in staged added lines"
        rc=1
    fi
    return $rc
}

# Comment lines are what git strips before storing the message, so they are
# ignored here; everything else counts.
check_commit_message() {
    local file=$1 text subject rest rc=0
    text=$(grep -v '^#' "$file" | tr -d '\r')
    subject=$(printf '%s\n' "$text" | grep -m 1 .)
    rest=$(printf '%s\n' "$text" | grep . | tail -n +2)
    if [ -n "$rest" ]; then
        echo "commit message must be one line with no body"
        rc=1
    fi
    if printf '%s\n' "$rest" | grep -q -E '^[A-Za-z][A-Za-z-]*: ' ||
        printf '%s\n' "$text" | grep -q -i 'co-authored-by'; then
        echo "commit message contains a trailer"
        rc=1
    fi
    if ! printf '%s\n' "$subject" | grep -q -E "$COMMIT_SUBJECT_RE"; then
        echo "subject must start with feat, fix, docs, test, refactor or chore, then (scope), then ': '"
        rc=1
    fi
    if printf '%s\n' "$text" | grep -q -F -e "$EM_DASH" -e "$EN_DASH"; then
        echo "commit message contains an em or en dash"
        rc=1
    fi
    if printf '%s\n' "$text" | grep -q -i -E "$BANNED_NAME_RE"; then
        echo "commit message contains the banned name"
        rc=1
    fi
    return $rc
}
