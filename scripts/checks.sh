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

# Every lockfile source is crates.io or a pinned ChronoCoders git commit; every git dependency in
# the manifest is pinned to a full rev; no [patch] or [replace] can redirect a crates.io name; and
# git-sourced packages get the advisory check that cargo audit skips for git sources. Arguments:
# manifest path, lockfile path, so the check can run on fixtures.
check_git_dependencies() {
    python3 -I - "$1" "$2" <<'PY'
import json
import os
import re
import subprocess
import sys
import tempfile
import tomllib

CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
GIT_BASE = re.compile(r"^https://github\.com/ChronoCoders/[A-Za-z0-9._-]+$")
FULL_REV = re.compile(r"^[0-9a-f]{40}$")
DEP_TABLES = ("dependencies", "dev-dependencies", "build-dependencies",
              "dev_dependencies", "build_dependencies")

manifest_path, lock_path = sys.argv[1], sys.argv[2]
problems = []


def git_source_problem(source):
    url = source[len("git+"):]
    url, _, fragment = url.partition("#")
    base, has_query, query = url.partition("?")
    if not GIT_BASE.match(base):
        return f"host or owner is not github.com/ChronoCoders: {base}"
    if not has_query:
        return "no ?rev= query"
    key, _, value = query.partition("=")
    if key in ("branch", "tag"):
        return f"pinned by {key}, not rev"
    if key != "rev" or "&" in value:
        return f"query is not a single rev: {query}"
    if not FULL_REV.match(value):
        return f"rev is not 40 lowercase hex: {value}"
    if fragment != value:
        return f"fragment {fragment or '(none)'} differs from rev {value}"
    return None


with open(lock_path, "rb") as f:
    lock = tomllib.load(f)
git_packages = []
for pkg in lock.get("package", []):
    source = pkg.get("source")
    if source is None or source == CRATES_IO:
        continue
    where = f"Cargo.lock {pkg['name']} {pkg['version']}"
    if not source.startswith("git+"):
        problems.append(f"{where}: registry is not crates.io: {source}")
        continue
    problem = git_source_problem(source)
    if problem:
        problems.append(f"{where}: {problem}")
    git_packages.append((pkg["name"], pkg["version"], source))

with open(manifest_path, "rb") as f:
    manifest = tomllib.load(f)
for section in ("patch", "replace"):
    if section in manifest:
        problems.append(f"Cargo.toml has a [{section}] section: {sorted(manifest[section])}")
tables = [(name, manifest.get(name, {})) for name in DEP_TABLES]
for cfg, target in manifest.get("target", {}).items():
    tables += [(f"target.{cfg}.{name}", target.get(name, {})) for name in DEP_TABLES]
tables.append(("workspace.dependencies", manifest.get("workspace", {}).get("dependencies", {})))
for table, deps in tables:
    for dep, spec in deps.items():
        if not isinstance(spec, dict) or "git" not in spec:
            continue
        where = f"Cargo.toml [{table}] {dep}"
        for key in ("branch", "tag"):
            if key in spec:
                problems.append(f"{where}: pinned by {key}")
        if not FULL_REV.match(str(spec.get("rev", ""))):
            problems.append(f"{where}: rev is not 40 lowercase hex: {spec.get('rev', '(none)')}")

# cargo audit matches advisories only for crates.io sources. A copy of the lockfile with the git
# sources rewritten to crates.io lets its own matcher check the git packages by name and version.
if git_packages:
    with open(lock_path) as f:
        text = f.read()
    text = re.sub(r'(?m)^source = "git\+[^"]*"$', f'source = "{CRATES_IO}"', text)
    with tempfile.NamedTemporaryFile("w", suffix=".lock", delete=False) as tmp:
        tmp.write(text)
    try:
        run = subprocess.run(["cargo", "audit", "--json", "--file", tmp.name],
                             capture_output=True, text=True)
    finally:
        os.unlink(tmp.name)
    try:
        report = json.loads(run.stdout)
    except json.JSONDecodeError:
        print(run.stderr.strip())
        problems.append("cargo audit produced no report for the git packages")
        report = None
    if report is not None:
        wanted = {(name, version) for name, version, _ in git_packages}
        findings = [("vulnerability", v) for v in report["vulnerabilities"]["list"]]
        findings += [(kind, w) for kind, ws in report["warnings"].items() for w in ws]
        for kind, finding in findings:
            pkg = finding["package"]
            if (pkg["name"], pkg["version"]) in wanted:
                advisory = (finding.get("advisory") or {}).get("id", "no advisory id")
                problems.append(f"git package {pkg['name']} {pkg['version']}: {kind} {advisory}")

for problem in problems:
    print(problem)
if problems:
    sys.exit(1)
if git_packages:
    listed = ", ".join(f"{name} {version} ({source})" for name, version, source in git_packages)
    print(f"git dependencies: {len(git_packages)}: {listed}")
else:
    print("git dependencies: none")
PY
}
