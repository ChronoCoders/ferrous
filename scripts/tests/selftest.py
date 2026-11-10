"""Self-test of every gate check: scripts/checks.sh, the hooks in .githooks/ and release_age.py.

Usage: selftest.py <repository root>. Every case builds its fixture in a fresh temporary directory
and asserts the exit status and the message of the branch it targets. Hook cases run real git
commits and pushes in scratch repositories and also assert the receipt: HEAD or the remote ref.
Fixtures spell the banned name, the dashes and CR with escapes so this file never matches the
gate's own file checks. run.sh runs this in a network namespace with no network.
"""

import datetime
import importlib.util
import os
import re
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from collections.abc import Callable
from functools import partial
from types import ModuleType
from typing import Any

ROOT = os.path.abspath(sys.argv[1])
CHECKS = os.path.join(ROOT, "scripts", "checks.sh")
HOOKS = os.path.join(ROOT, ".githooks")
RELEASE_AGE = os.path.join(ROOT, "scripts", "release_age.py")
ADVISORY_DB = os.path.join(os.path.expanduser("~"), ".cargo", "advisory-db")

EM = chr(0x2014)
EN = chr(0x2013)
CR = "\r"
NAME = "c\x6caude"
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
REV = "0123456789abcdef0123456789abcdef01234567"
ZERO = "0" * 40

# A hook exports GIT_DIR and similar variables, and the gate runs this from pre-push, so every git
# variable is dropped: a scratch command must never reach the real repository.
GIT_ENV = dict(
    {k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
    GIT_CONFIG_GLOBAL=os.devnull,
    GIT_CONFIG_NOSYSTEM="1",
    GIT_AUTHOR_NAME="ctl",
    GIT_AUTHOR_EMAIL="ctl@localhost",
    GIT_COMMITTER_NAME="ctl",
    GIT_COMMITTER_EMAIL="ctl@localhost",
    https_proxy="http://127.0.0.1:9",
    HTTPS_PROXY="http://127.0.0.1:9",
)

Result = tuple[int, str]
CASES: list[tuple[str, str, Callable[[], Result], int, list[str] | None]] = []


def case(group: str, name: str, fn: Callable[[], Result], rc: int, want: list[str] | None) -> None:
    """want lists substrings the output must contain; None means the output must be empty."""
    CASES.append((group, name, fn, rc, want))


def run(args: list[str], cwd: str, stdin: str | None = None) -> Result:
    p = subprocess.run(
        args, cwd=cwd, input=stdin, capture_output=True, text=True, env=GIT_ENV, check=False
    )
    return p.returncode, (p.stdout + p.stderr).strip()


def git(cwd: str, *args: str) -> str:
    rc, out = run(["git", *args], cwd)
    if rc != 0:
        raise RuntimeError(f"fixture git {' '.join(args)} failed: {out}")
    return out


def checks(cwd: str, call: str, stdin: str | None = None) -> Result:
    return run(["bash", "-c", f'. "$0"; {call}', CHECKS], cwd, stdin)


def write(root: str, path: str, text: str) -> None:
    full = os.path.join(root, path)
    os.makedirs(os.path.dirname(full) or root, exist_ok=True)
    with open(full, "w", newline="") as f:
        f.write(text)


def repo(d: str, files: dict[str, str], message: str = "test(ctl): base") -> str:
    git(d, "init", "-q", "-b", "main")
    for path, text in files.items():
        write(d, path, text)
    git(d, "add", "-A")
    git(d, "commit", "-q", "--allow-empty", "-m", message)
    return d


def in_repo(
    files: dict[str, str], call: str, after: Callable[[str], object] | None = None
) -> Result:
    with tempfile.TemporaryDirectory() as d:
        repo(d, files)
        if after:
            after(d)
        return checks(d, call)


# Tracked-file checks: dashes, CR, banned name.

for check, label, msg, bad in [
    ("check_tracked_dashes", "em_dash", "em or en dash found in tracked files", f"a {EM} b\n"),
    ("check_tracked_dashes", "en_dash", "em or en dash found in tracked files", f"1{EN}2\n"),
    ("check_tracked_line_endings", "crlf", "CR found in tracked files", f"a{CR}\nb\n"),
    ("check_tracked_line_endings", "lone_cr", "CR found in tracked files", f"a{CR}b\n"),
    ("check_tracked_banned_name", "lower", "banned name found in tracked files", f"{NAME}\n"),
    ("check_tracked_banned_name", "upper", "banned name found in tracked files", NAME.upper()),
    ("check_tracked_banned_name", "title", "banned name found in tracked files", NAME.title()),
]:
    case(check, label, partial(in_repo, {"a.txt": bad}, check), 1, [msg])
# git grep searches content only, so a name in a path alone passes this check; the pre-commit path
# check is what refuses it. The case pins that division of work.
case(
    "check_tracked_banned_name",
    "path_only_passes",
    lambda: in_repo({f"{NAME}.txt": "x\n"}, "check_tracked_banned_name"),
    0,
    None,
)


def untracked_everything(d: str) -> None:
    write(d, "u.txt", f"{EM}{EN}{CR}{NAME}\n")


for check in ["check_tracked_dashes", "check_tracked_line_endings", "check_tracked_banned_name"]:
    case(check, "clean", partial(in_repo, {"a.txt": "plain - text\n"}, check), 0, None)
    case(
        check,
        "untracked_file_ignored",
        partial(in_repo, {"a.txt": "x\n"}, check, untracked_everything),
        0,
        None,
    )


# Commit-message banned name and the gate's combined banned-name step.


def message_repo(d: str, messages: list[str]) -> None:
    repo(d, {"a.txt": "x\n"})
    git(d, "update-ref", "refs/remotes/origin/main", "HEAD")
    for m in messages:
        git(d, "commit", "-q", "--allow-empty", "-m", m)


def message_case(messages: list[str], call: str, origin: bool = True) -> Result:
    with tempfile.TemporaryDirectory() as d:
        message_repo(d, messages)
        if not origin:
            git(d, "update-ref", "-d", "refs/remotes/origin/main")
        return checks(d, call)


MSG_HIT = "banned name found in commit messages in origin/main..HEAD"
for label, msgs in [
    ("subject", [f"fix(x): add {NAME}"]),
    ("body", [f"fix(x): add y\n\nbody {NAME.upper()}"]),
    ("second_of_two", ["fix(x): first", f"fix(x): {NAME.title()}"]),
]:
    case(
        "check_message_banned_name",
        label,
        partial(message_case, msgs, "check_message_banned_name origin/main..HEAD"),
        1,
        [MSG_HIT],
    )
case(
    "check_message_banned_name",
    "clean",
    lambda: message_case(["fix(x): fine"], "check_message_banned_name origin/main..HEAD"),
    0,
    None,
)
case(
    "check_message_banned_name",
    "before_range_ignored",
    lambda: message_case([], "check_message_banned_name origin/main..HEAD"),
    0,
    None,
)
case(
    "check_banned_name_since_origin",
    "clean",
    lambda: message_case(["fix(x): fine"], "check_banned_name_since_origin"),
    0,
    None,
)
case(
    "check_banned_name_since_origin",
    "no_origin_main",
    lambda: message_case(["fix(x): fine"], "check_banned_name_since_origin", origin=False),
    1,
    ["origin/main not found; cannot check commit messages"],
)
case(
    "check_banned_name_since_origin",
    "message_hit",
    lambda: message_case([f"fix(x): {NAME}"], "check_banned_name_since_origin"),
    1,
    [MSG_HIT],
)
case(
    "check_banned_name_since_origin",
    "tracked_hit",
    lambda: in_repo({"a.txt": f"{NAME}\n"}, "check_banned_name_since_origin"),
    1,
    ["banned name found in tracked files"],
)


# Crate attributes.

FORBID = "#![forbid(unsafe_code)]"
DENY = "#![deny(warnings)]"
for label, text, rc, want in [
    ("both_present", f"{FORBID}\n{DENY}\n", 0, None),
    ("forbid_missing", f"{DENY}\n", 1, [f"src/lib.rs is missing {FORBID}"]),
    ("deny_missing", f"{FORBID}\n", 1, [f"src/lib.rs is missing {DENY}"]),
    (
        "both_missing",
        "pub fn f() {}\n",
        1,
        [f"src/lib.rs is missing {FORBID}", f"src/lib.rs is missing {DENY}"],
    ),
    ("forbid_indented", f"  {FORBID}\n{DENY}\n", 1, [f"src/lib.rs is missing {FORBID}"]),
    ("deny_in_comment", f"{FORBID}\n// {DENY}\n", 1, [f"src/lib.rs is missing {DENY}"]),
]:
    case(
        "check_crate_attributes",
        label,
        partial(in_repo, {"src/lib.rs": text}, "check_crate_attributes"),
        rc,
        want,
    )


# Staged checks used by pre-commit.


def staged(
    files: dict[str, str],
    call: str,
    stage: Callable[[str], object],
    base: dict[str, str] | None = None,
) -> Result:
    with tempfile.TemporaryDirectory() as d:
        repo(d, base or {"a.txt": "x\n"})
        for path, text in files.items():
            write(d, path, text)
        stage(d)
        return checks(d, call)


def add_all(d: str) -> None:
    git(d, "add", "-A")


LOCAL = "local-only paths are staged; unstage them"
for label, path in [
    ("root_md", f"{NAME.upper()}.md"),
    ("root_md_lower", f"{NAME}.md"),
    ("nested_md", f"sub/{NAME.upper()}.md"),
    ("settings_dir", f".{NAME}/settings.json"),
    ("private_dir", "private/DECISIONS.md"),
    ("cargo_config", ".cargo/config"),
    ("cargo_config_toml", ".cargo/config.toml"),
    ("nested_cargo_config", "sub/.cargo/config.toml"),
]:
    case(
        "check_staged_paths",
        label,
        partial(staged, {path: "x\n"}, "check_staged_paths", add_all),
        1,
        [LOCAL],
    )
for label, path in [
    ("ordinary_file", "src/a.rs"),
    ("private_prefix_not_dir", "privateer.md"),
    ("md_suffix_only", f"not{NAME}.md.bak"),
    ("cargo_other_file", ".cargo/credentials.toml.example"),
]:
    case(
        "check_staged_paths",
        f"pass_{label}",
        partial(staged, {path: "x\n"}, "check_staged_paths", add_all),
        0,
        None,
    )
case(
    "check_staged_paths",
    "pass_deletion_not_flagged",
    lambda: staged(
        {},
        "check_staged_paths",
        lambda d: git(d, "rm", "-q", "--cached", "private/x.md"),
        base={"private/x.md": "x\n"},
    ),
    0,
    None,
)
case(
    "check_staged_banned_name",
    "path",
    lambda: staged({f"{NAME}_notes.txt": "x\n"}, "check_staged_banned_name", add_all),
    1,
    ["banned name in staged paths"],
)
case(
    "check_staged_banned_name",
    "added_line",
    lambda: staged({"a.txt": f"x\n{NAME.title()}\n"}, "check_staged_banned_name", add_all),
    1,
    ["banned name in staged added lines"],
)
case(
    "check_staged_banned_name",
    "path_and_line",
    lambda: staged({f"{NAME}.txt": f"{NAME}\n"}, "check_staged_banned_name", add_all),
    1,
    ["banned name in staged paths", "banned name in staged added lines"],
)
case(
    "check_staged_banned_name",
    "removed_line_passes",
    lambda: staged(
        {"a.txt": "x\n"}, "check_staged_banned_name", add_all, base={"a.txt": f"x\n{NAME}\n"}
    ),
    0,
    None,
)
for label, ch in [("em", EM), ("en", EN)]:
    case(
        "check_staged_dashes",
        label,
        partial(staged, {"a.txt": f"x\ny {ch} z\n"}, "check_staged_dashes", add_all),
        1,
        ["em or en dash in staged added lines"],
    )
case(
    "check_staged_dashes",
    "removed_line_passes",
    lambda: staged({"a.txt": "x\n"}, "check_staged_dashes", add_all, base={"a.txt": f"x\n{EM}\n"}),
    0,
    None,
)
case(
    "check_staged_dashes",
    "unstaged_dash_passes",
    lambda: staged({"a.txt": f"{EM}\n"}, "check_staged_dashes", lambda d: None),
    0,
    None,
)


# Commit message.


def message(text: str) -> Result:
    with tempfile.TemporaryDirectory() as d:
        write(d, "MSG", text)
        return checks(d, f'check_commit_message "{d}/MSG"')


SUBJECT = (
    "subject must start with feat, fix, docs, test, refactor or chore, then (scope), then ': '"
)
for prefix in ["feat", "fix", "docs", "test", "refactor", "chore"]:
    case(
        "check_commit_message",
        f"pass_{prefix}",
        partial(message, f"{prefix}(gate): do a thing\n"),
        0,
        None,
    )
for label, text, want in [
    ("pass_scope_chars", "fix(src/net_v2.rs-x): y\n", None),
    ("pass_comment_lines_ignored", "fix(x): y\n# Please enter the message\n# more\n", None),
    ("pass_blank_lines_ignored", "\nfix(x): y\n\n\n", None),
    ("pass_crlf_stripped", f"fix(x): y{CR}\n", None),
    ("body", "fix(x): y\n\nmore text\n", ["commit message must be one line with no body"]),
    (
        "trailer_in_body",
        "fix(x): y\n\nSigned-off-by: a\n",
        ["commit message must be one line with no body", "commit message contains a trailer"],
    ),
    ("co_authored_in_subject", "fix(x): note Co-Authored-By here\n", ["contains a trailer"]),
    ("unknown_type", "feature(x): y\n", [SUBJECT]),
    ("uppercase_type", "Fix(x): y\n", [SUBJECT]),
    ("no_scope", "fix: y\n", [SUBJECT]),
    ("empty_scope", "fix(): y\n", [SUBJECT]),
    ("uppercase_scope", "fix(Gate): y\n", [SUBJECT]),
    ("no_space", "fix(x):y\n", [SUBJECT]),
    ("double_space", "fix(x):  y\n", [SUBJECT]),
    ("empty_message", "# only a comment\n", [SUBJECT]),
    ("em_dash", f"fix(x): a {EM} b\n", ["commit message contains an em or en dash"]),
    ("en_dash", f"fix(x): 1{EN}2\n", ["commit message contains an em or en dash"]),
    ("banned_name", f"fix(x): {NAME.upper()}\n", ["commit message contains the banned name"]),
]:
    case("check_commit_message", label, partial(message, text), 0 if want is None else 1, want)


# cargo test output parser.

OK5 = "test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.1s"
for label, text, rc_in, rc, want in [
    ("pass", f"{OK5}\n{OK5}\n", 0, 0, ["tests: 10 passed, 0 ignored across 2 test result lines"]),
    (
        "filtered_out_is_not_ignored",
        "test result: ok. 3 passed; 0 failed; 0 ignored; 0 measured; 9 filtered out\n",
        0,
        0,
        ["tests: 3 passed, 0 ignored across 1 test result lines"],
    ),
    (
        "ignored",
        f"{OK5}\ntest result: ok. 4 passed; 0 failed; 2 ignored; 0 measured; 0 filtered out\n",
        0,
        1,
        ["tests: 9 passed, 2 ignored", "1 test result lines report ignored tests"],
    ),
    ("no_result_lines", "Compiling x\n", 0, 1, ["no test result lines found in cargo test output"]),
    ("cargo_failed", f"{OK5}\n", 101, 1, ["tests: 5 passed, 0 ignored across 1"]),
]:
    case(
        "check_cargo_test_output",
        label,
        partial(checks, ROOT, f"check_cargo_test_output {rc_in}", text),
        rc,
        want,
    )


# cargo audit output parser.


def block(crate: str, version: str, aid: str | None, kind: str | None) -> str:
    lines = [f"Crate:     {crate}", f"Version:   {version}"]
    if kind:
        lines.append(f"Warning:   {kind}")
    lines.append("Title:     something")
    if aid:
        lines.append(f"ID:        {aid}")
    return "\n".join(lines) + "\n"


BINCODE = block("bincode", "1.3.3", "RUSTSEC-2025-0141", "unmaintained")
CLEAR = block("clear_on_drop", "0.2.5", "RUSTSEC-2026-0283", "unmaintained")
DALEK = block("curve25519-dalek", "4.1.2", "RUSTSEC-2024-0344", None)
LRU = block("lru", "0.12.5", "RUSTSEC-2026-0002", None)
for label, text, rc_in, rc, want in [
    ("clean", "Scanning Cargo.lock\n", 0, 0, ["audit: 0 vulnerabilities, 0 warnings"]),
    (
        "one_warning",
        BINCODE + "warning: 1 allowed warning found\n",
        0,
        0,
        [
            "warning: RUSTSEC-2025-0141 bincode 1.3.3 unmaintained",
            "audit: 0 vulnerabilities, 1 warning",
        ],
    ),
    ("two_warnings", BINCODE + CLEAR, 0, 0, ["audit: 0 vulnerabilities, 2 warnings"]),
    (
        "warning_without_id",
        block("der", "0.8.0", None, "yanked"),
        0,
        0,
        ["warning: none der 0.8.0 yanked"],
    ),
    (
        "one_vulnerability",
        DALEK + "error: 1 vulnerability found!\n",
        1,
        1,
        ["vulnerability: RUSTSEC-2024-0344 curve25519-dalek 4.1.2", "audit: 1 vulnerability, 0 w"],
    ),
    (
        "two_vulnerabilities",
        DALEK + LRU + "error: 2 vulnerabilities found!\n",
        1,
        1,
        ["audit: 2 vulnerabilities, 0 warnings"],
    ),
    (
        "vulnerability_after_warnings",
        DALEK + BINCODE + "error: 1 vulnerability found!\n",
        1,
        1,
        [
            (
                "warning: RUSTSEC-2025-0141 bincode 1.3.3 unmaintained\n"
                "vulnerability: RUSTSEC-2024-0344 curve25519-dalek 4.1.2\n"
                "audit: 1 vulnerability, 1 warning"
            )
        ],
    ),
    (
        "count_mismatch",
        DALEK,
        1,
        1,
        ["audit output could not be parsed: 1 vulnerability blocks, cargo audit reports 0"],
    ),
    ("audit_failed_no_blocks", "error: fetch failed\n", 1, 1, ["audit: 0 vulnerabilities"]),
    (
        "vulnerability_with_exit_0",
        DALEK + "error: 1 vulnerability found!\n",
        0,
        1,
        ["vulnerability: RUSTSEC-2024-0344"],
    ),
]:
    case(
        "check_cargo_audit_output",
        label,
        partial(checks, ROOT, f"check_cargo_audit_output {rc_in}", text),
        rc,
        want,
    )


# Git dependencies: the 35 commit-4 controls, the reviewer's look-alike registry, and the rest of
# the branches. Advisory lookups use the local advisory database with no fetch.


def read(path: str) -> str:
    with open(path) as f:
        return f.read()


REAL_TOML = read(os.path.join(ROOT, "Cargo.toml"))
REAL_LOCK = read(os.path.join(ROOT, "Cargo.lock"))
GOOD_GIT = f"git+https://github.com/ChronoCoders/fixture?rev={REV}#{REV}"


def append_pkg(lock: str, name: str, version: str, source: str) -> str:
    pkg = f'[[package]]\nname = "{name}"\nversion = "{version}"\nsource = "{source}"\n'
    return lock.rstrip("\n") + "\n\n" + pkg


def set_source(lock: str, name: str, version: str, source: str) -> str:
    head = f'name = "{name}"\nversion = "{version}"\n'
    i = lock.index(head)
    j = lock.index("\n\n", i) if "\n\n" in lock[i:] else len(lock)
    blk = re.sub(r'(?m)^source = "[^"]*"$', f'source = "{source}"', lock[i:j])
    blk = re.sub(r"(?m)^checksum = .*\n?", "", blk)
    return lock[:i] + blk + lock[j:]


def manifest(extra: str) -> str:
    return '[package]\nname = "fixture"\nversion = "0.0.0"\nedition = "2021"\n\n' + extra


def gitdeps(toml: str, lock: str, db: str = ADVISORY_DB) -> Result:
    with tempfile.TemporaryDirectory() as d:
        write(d, "Cargo.toml", toml)
        write(d, "Cargo.lock", lock)
        return checks(d, f'check_git_dependencies Cargo.toml Cargo.lock "{db}"')


def lock_case(label: str, source: str, rc: int, want: str) -> None:
    lock = append_pkg(REAL_LOCK, "fixture-crate", "1.0.0", source)
    case("check_git_dependencies", label, lambda: gitdeps(REAL_TOML, lock), rc, [want])


GH = "git+https://github.com/ChronoCoders/fixture"
lock_case("r1_allowed_crates_io_registry", CRATES_IO, 0, "git dependencies: none")
lock_case(
    "r1_foreign_sparse_crates_io_alias",
    "sparse+https://index.crates.io/",
    1,
    "registry is not crates.io: sparse+https://index.crates.io/",
)
lock_case(
    "r1_foreign_registry",
    "registry+https://example.com/index",
    1,
    "registry is not crates.io: registry+https://example.com/index",
)
lock_case(
    "r1_lookalike_crates_io_registry",
    CRATES_IO + "-evil",
    1,
    f"registry is not crates.io: {CRATES_IO}-evil",
)
lock_case("r2_pass_pinned_chronocoders_rev", GOOD_GIT, 0, "git dependencies: 1: fixture-crate")
lock_case(
    "r2_other_host",
    f"git+https://gitlab.com/ChronoCoders/fixture?rev={REV}#{REV}",
    1,
    "host or owner is not github.com/ChronoCoders",
)
lock_case(
    "r2_other_owner",
    f"git+https://github.com/zkcrypto/fixture?rev={REV}#{REV}",
    1,
    "host or owner is not github.com/ChronoCoders",
)
lock_case("r2_branch", f"{GH}?branch=main#{REV}", 1, "pinned by branch, not rev")
lock_case("r2_tag", f"{GH}?tag=v5.0.0#{REV}", 1, "pinned by tag, not rev")
lock_case("r2_no_query", f"{GH}#{REV}", 1, "no ?rev= query")
lock_case("r2_other_query_key", f"{GH}?commit={REV}#{REV}", 1, "query is not a single rev")
lock_case("r2_two_query_keys", f"{GH}?rev={REV}&x=1#{REV}", 1, "query is not a single rev")
lock_case("r2_short_rev", f"{GH}?rev={REV[:7]}#{REV}", 1, "rev is not 40 lowercase hex")
lock_case(
    "r2_uppercase_rev", f"{GH}?rev={REV.upper()}#{REV.upper()}", 1, "rev is not 40 lowercase hex"
)
lock_case("r2_fragment_differs", f"{GH}?rev={REV}#{'f' * 40}", 1, "differs from rev")
lock_case("r2_fragment_missing", f"{GH}?rev={REV}", 1, "fragment (none) differs from rev")

GIT_NO_REV = '{ git = "https://github.com/ChronoCoders/fixture" }'
GIT_REV = f'{{ git = "https://github.com/ChronoCoders/fixture", rev = "{REV}" }}'
for table in [
    "dependencies",
    "dev-dependencies",
    "build-dependencies",
    "dev_dependencies",
    "build_dependencies",
    "target.'cfg(unix)'.dependencies",
    "target.'cfg(unix)'.dev-dependencies",
    "target.'cfg(unix)'.build-dependencies",
    "workspace.dependencies",
]:
    case(
        "check_git_dependencies",
        f"r3_no_rev_in_[{table}]",
        partial(gitdeps, manifest(f"[{table}]\nfixture = {GIT_NO_REV}\n"), REAL_LOCK),
        1,
        ["rev is not 40 lowercase hex: (none)"],
    )
for label, extra, rc, want_msg in [
    (
        "r3_no_rev_dotted_table",
        '[dependencies.fixture]\ngit = "https://github.com/ChronoCoders/fixture"\n',
        1,
        "[dependencies] fixture: rev is not 40 lowercase hex",
    ),
    (
        "r3_branch",
        (
            f'[dependencies]\nfixture = {{ git = "https://github.com/ChronoCoders/fixture", '
            f'rev = "{REV}", branch = "main" }}\n'
        ),
        1,
        "pinned by branch",
    ),
    (
        "r3_tag",
        (
            f'[dependencies]\nfixture = {{ git = "https://github.com/ChronoCoders/fixture", '
            f'rev = "{REV}", tag = "v1" }}\n'
        ),
        1,
        "pinned by tag",
    ),
    (
        "r3_short_rev",
        (
            f'[dependencies]\nfixture = {{ git = "https://github.com/ChronoCoders/fixture", '
            f'rev = "{REV[:7]}" }}\n'
        ),
        1,
        f"rev is not 40 lowercase hex: {REV[:7]}",
    ),
    ("r3_pass_full_rev", f"[dependencies]\nfixture = {GIT_REV}\n", 0, "git dependencies: none"),
    (
        "r4_patch_crates_io",
        '[patch.crates-io]\nbulletproofs = { path = "../bp" }\n',
        1,
        "has a [patch] section: ['crates-io']",
    ),
    (
        "r4_patch_git_url",
        '[patch."https://github.com/zkcrypto/bulletproofs"]\nbulletproofs = { path = "../bp" }\n',
        1,
        "has a [patch] section: ['https://github.com/zkcrypto/bulletproofs']",
    ),
    (
        "r4_replace",
        '[replace]\n"bulletproofs:5.0.0" = { path = "../bp" }\n',
        1,
        "has a [replace] section",
    ),
]:
    case(
        "check_git_dependencies",
        label,
        partial(gitdeps, manifest(extra), REAL_LOCK),
        rc,
        [want_msg],
    )


def chrono(name: str) -> str:
    return f"git+https://github.com/ChronoCoders/{name}?rev={REV}#{REV}"


for label, lock, rc, want_msg in [
    (
        "r5_unmaintained_clear_on_drop",
        set_source(REAL_LOCK, "clear_on_drop", "0.2.5", chrono("clear_on_drop")),
        1,
        "git package clear_on_drop 0.2.5: unmaintained RUSTSEC-2026-0283",
    ),
    (
        "r5_vulnerability_curve25519_dalek_4_1_2",
        append_pkg(REAL_LOCK, "curve25519-dalek", "4.1.2", chrono("curve25519-dalek")),
        1,
        "git package curve25519-dalek 4.1.2: vulnerability RUSTSEC-2024-0344",
    ),
    (
        "r5_unsound_lru_0_12_5",
        append_pkg(REAL_LOCK, "lru", "0.12.5", chrono("lru")),
        1,
        "git package lru 0.12.5: unsound RUSTSEC-2026-0",
    ),
    (
        "r5_yanked_der_0_8_0",
        append_pkg(REAL_LOCK, "der", "0.8.0", chrono("der")),
        1,
        "git package der 0.8.0: yanked",
    ),
    (
        "r5_clean_git_package_passes",
        append_pkg(REAL_LOCK, "itoa", "1.0.15", chrono("itoa")),
        0,
        "git dependencies: 1: itoa 1.0.15",
    ),
]:
    case("check_git_dependencies", label, partial(gitdeps, REAL_TOML, lock), rc, [want_msg])
case(
    "check_git_dependencies",
    "r5_no_audit_report",
    lambda: gitdeps(
        REAL_TOML,
        append_pkg(REAL_LOCK, "itoa", "1.0.15", chrono("itoa")),
        db="/nonexistent/advisory-db",
    ),
    1,
    ["cargo audit produced no report for the git packages"],
)
case(
    "check_git_dependencies",
    "r6_real_repository",
    lambda: gitdeps(REAL_TOML, REAL_LOCK),
    0,
    ["git dependencies: none"],
)


# Hooks, run by real git commands in scratch repositories. The scratch gate.sh is a stub, so the
# pre-push cases test the hook and not the gate it hands over to.

STUB_GATE = '#!/usr/bin/env bash\necho "stub gate ran"\nexit "${STUB_GATE_RC:-0}"\n'


def hook_repo(d: str, gate_rc: int = 0) -> str:
    work = os.path.join(d, "work")
    bare = os.path.join(d, "remote.git")
    os.makedirs(work)
    git(d, "init", "-q", "--bare", bare)
    shutil.copytree(HOOKS, os.path.join(work, ".githooks"))
    os.makedirs(os.path.join(work, "scripts"))
    shutil.copy2(CHECKS, os.path.join(work, "scripts", "checks.sh"))
    write(work, "gate.sh", STUB_GATE.replace("${STUB_GATE_RC:-0}", str(gate_rc)))
    os.chmod(os.path.join(work, "gate.sh"), 0o755)
    repo(work, {"README": "base\n"})
    git(work, "remote", "add", "bare", bare)
    git(work, "push", "-q", "bare", "main")
    git(work, "config", "core.hooksPath", ".githooks")
    return work


def head(work: str) -> str:
    return git(work, "rev-parse", "HEAD")


def remote_ref(work: str, ref: str = "refs/heads/main") -> str:
    rc, out = run(
        ["git", "--git-dir", os.path.join(work, "..", "remote.git"), "rev-parse", ref], work
    )
    return out if rc == 0 else "(none)"


def commit_case(files: dict[str, str], msg: str) -> Result:
    with tempfile.TemporaryDirectory() as d:
        work = hook_repo(d)
        before = head(work)
        for path, text in files.items():
            write(work, path, text)
        git(work, "add", "-A")
        rc, out = run(["git", "commit", "-q", "-m", msg], work)
        moved = head(work) != before
        return rc, out + f"\nreceipt: HEAD {'advanced' if moved else 'unchanged'}"


ADVANCED = "receipt: HEAD advanced"
UNCHANGED = "receipt: HEAD unchanged"
case("pre-commit", "pass", lambda: commit_case({"a.txt": "x\n"}, "fix(x): y"), 0, [ADVANCED])
for label, files, hook_msg in [
    ("local_only_path", {"private/x.md": "x\n"}, LOCAL),
    ("banned_name_path", {f"{NAME}.txt": "x\n"}, "banned name in staged paths"),
    ("banned_name_line", {"a.txt": f"{NAME}\n"}, "banned name in staged added lines"),
    ("em_dash", {"a.txt": f"a {EM} b\n"}, "em or en dash in staged added lines"),
    ("en_dash", {"a.txt": f"1{EN}2\n"}, "em or en dash in staged added lines"),
]:
    case("pre-commit", label, partial(commit_case, files, "fix(x): y"), 1, [hook_msg, UNCHANGED])
case(
    "pre-commit",
    "all_checks_run",
    lambda: commit_case({"private/x.md": f"{NAME} {EM}\n"}, "fix(x): y"),
    1,
    [LOCAL, "banned name in staged added lines", "em or en dash in staged added lines", UNCHANGED],
)
case("commit-msg", "pass", lambda: commit_case({"a.txt": "x\n"}, "docs(x): y"), 0, [ADVANCED])
for label, msg, hook_msg in [
    ("body", "fix(x): y\n\nbody", "commit message must be one line with no body"),
    ("trailer", "fix(x): y\n\nCo-authored-by: a <a@b>", "commit message contains a trailer"),
    ("subject", "update things", SUBJECT),
    ("dash", f"fix(x): a {EM} b", "commit message contains an em or en dash"),
    ("banned_name", f"fix(x): {NAME}", "commit message contains the banned name"),
]:
    case(
        "commit-msg",
        label,
        partial(commit_case, {"a.txt": "x\n"}, msg),
        1,
        [hook_msg, UNCHANGED],
    )


def push_case(
    setup: Callable[[str], object], push: list[str], gate_rc: int = 0, ref: str = "refs/heads/main"
) -> Result:
    with tempfile.TemporaryDirectory() as d:
        work = hook_repo(d, gate_rc)
        setup(work)
        before = remote_ref(work, ref)
        rc, out = run(["git", "push", "-q", "bare", *push], work)
        after = remote_ref(work, ref)
        return rc, out + f"\nreceipt: {ref} {'moved' if after != before else 'unchanged'}"


def commits(n: int) -> Callable[[str], object]:
    def setup(work: str) -> None:
        for i in range(n):
            write(work, f"f{i}.txt", f"{i}\n")
            git(work, "add", "-A")
            git(work, "commit", "-q", "-m", f"fix(x): commit {i}")

    return setup


def then(*steps: Callable[[str], object]) -> Callable[[str], object]:
    def setup(work: str) -> None:
        for s in steps:
            s(work)

    return setup


MOVED = "receipt: refs/heads/main moved"
KEPT = "receipt: refs/heads/main unchanged"
case("pre-push", "pass", lambda: push_case(commits(1), ["main"]), 0, ["stub gate ran", MOVED])
case(
    "pre-push",
    "pass_untracked_file_allowed",
    lambda: push_case(then(commits(1), lambda w: write(w, "new.txt", "x\n")), ["main"]),
    0,
    ["stub gate ran", MOVED],
)
case(
    "pre-push",
    "gate_failure_blocks_push",
    lambda: push_case(commits(1), ["main"], gate_rc=1),
    1,
    ["stub gate ran", KEPT],
)
case(
    "pre-push",
    "two_new_commits",
    lambda: push_case(commits(2), ["main"]),
    1,
    ["exactly one new commit per push, got 2", KEPT],
)
case(
    "pre-push",
    "dirty_tracked_file",
    lambda: push_case(then(commits(1), lambda w: write(w, "README", "changed\n")), ["main"]),
    1,
    ["uncommitted changes to tracked files", KEPT],
)
case(
    "pre-push",
    "two_refs",
    lambda: push_case(then(commits(1), lambda w: git(w, "branch", "other")), ["main", "other"]),
    1,
    ["exactly one ref per push, got 2", KEPT],
)
case(
    "pre-push",
    "tag_push",
    lambda: push_case(then(commits(1), lambda w: git(w, "tag", "v1")), ["v1"], ref="refs/tags/v1"),
    1,
    ["tag push of refs/tags/v1", "receipt: refs/tags/v1 unchanged"],
)


def other_branch(work: str) -> None:
    bare = os.path.join(work, "..", "remote.git")
    git(work, "--git-dir", bare, "update-ref", "refs/heads/other", "refs/heads/main")


case(
    "pre-push",
    "ref_deletion",
    lambda: push_case(other_branch, [":other"], ref="refs/heads/other"),
    1,
    ["ref deletion of refs/heads/other", "receipt: refs/heads/other unchanged"],
)
case(
    "pre-push",
    "pushed_commit_not_head",
    lambda: push_case(
        then(
            lambda w: git(w, "checkout", "-q", "-b", "side"),
            commits(1),
            lambda w: git(w, "checkout", "-q", "main"),
            lambda w: git(w, "commit", "-q", "--allow-empty", "-m", "fix(x): g"),
        ),
        ["side:main"],
    ),
    1,
    ["is not HEAD", KEPT],
)
case(
    "pre-push",
    "new_branch_without_origin_main",
    lambda: push_case(commits(1), ["main:feature"], ref="refs/heads/feature"),
    1,
    ["new branch refs/heads/feature and origin/main not found", "refs/heads/feature unchanged"],
)
case(
    "pre-push",
    "new_branch_with_origin_main",
    lambda: push_case(
        then(lambda w: git(w, "update-ref", "refs/remotes/origin/main", "HEAD"), commits(1)),
        ["main:feature"],
        ref="refs/heads/feature",
    ),
    0,
    ["stub gate ran", "receipt: refs/heads/feature moved"],
)


def hook_stdin(stdin: str, setup: Callable[[str], object] | None = None) -> Result:
    """For pushes git would not hand to the hook without a forced push: the hook is run directly."""
    with tempfile.TemporaryDirectory() as d:
        work = hook_repo(d)
        if setup:
            setup(work)
        line = stdin.replace("HEAD_SHA", head(work))
        if "OLD" in line:
            line = line.replace("OLD", git(work, "rev-parse", "refs/tmp/old"))
        return run([os.path.join(work, ".githooks", "pre-push"), "bare", "x"], work, line)


def diverge(work: str) -> None:
    git(work, "commit", "-q", "--allow-empty", "-m", "fix(x): old tip")
    git(work, "update-ref", "refs/tmp/old", "HEAD")
    git(work, "reset", "-q", "--hard", "HEAD~1")
    git(work, "commit", "-q", "--allow-empty", "-m", "fix(x): new tip")


case("pre-push", "zero_refs", lambda: hook_stdin(""), 1, ["exactly one ref per push, got 0"])
case(
    "pre-push",
    "non_fast_forward",
    lambda: hook_stdin(
        "refs/heads/main HEAD_SHA refs/heads/main OLD\n",
        diverge,
    ),
    1,
    ["non-fast-forward: refs/heads/main is at"],
)
case(
    "pre-push",
    "cannot_count_commits",
    lambda: hook_stdin(
        f"refs/heads/main HEAD_SHA refs/heads/feature {ZERO}\n",
        lambda w: write(w, ".git/refs/remotes/origin/main", "1" * 40 + "\n"),
    ),
    1,
    ["cannot count commits in"],
)


# release_age.py, offline: an injected fetch for the rules, a stubbed urlopen for the fetcher, and
# scratch repositories for main().


def load_release_age() -> ModuleType:
    spec = importlib.util.spec_from_file_location("release_age", RELEASE_AGE)
    assert spec and spec.loader
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


RA = load_release_age()
DAY = datetime.timedelta(days=1)


def release(created: str, yanked: bool = False) -> dict[str, Any]:
    return {"version": {"created_at": created, "yanked": yanked}}


def ra_check(pkgs: list[tuple[str, str, str]], answers: dict[tuple[str, str], Any]) -> Result:
    asked: list[str] = []

    def fetch(name: str, version: str) -> dict[str, Any] | None:
        asked.append(f"{name} {version}")
        answer = answers.get((name, version))
        if isinstance(answer, Exception):
            raise answer
        return answer

    lines, problems = RA.check(pkgs, fetch, RA.CUTOFF)
    out = "\n".join(lines + problems + [f"asked: {', '.join(asked) or '(none)'}"])
    return (1 if problems else 0), out


CUT = RA.CUTOFF.isoformat()
AFTER = (RA.CUTOFF + DAY).isoformat()
BEFORE = (RA.CUTOFF - DAY).isoformat()
P = ("serde", "1.0.0", CRATES_IO)
for label, answer, rc, want in [
    (
        "old_enough",
        release(f"{BEFORE}T23:42:45.916886Z"),
        0,
        [f"ok serde 1.0.0 published {BEFORE}"],
    ),
    (
        "exactly_on_cutoff",
        release(f"{CUT}T23:59:59.999+00:00"),
        0,
        [f"ok serde 1.0.0 published {CUT}"],
    ),
    (
        "on_cutoff_in_utc_from_other_zone",
        release(f"{AFTER}T01:00:00+02:00"),
        0,
        [f"ok serde 1.0.0 published {CUT}"],
    ),
    (
        "too_new",
        release(f"{AFTER}T00:00:00+00:00"),
        1,
        [f"serde 1.0.0: published {AFTER}, after the cutoff {CUT}"],
    ),
    (
        "yanked",
        release(f"{BEFORE}T00:00:00+00:00", yanked=True),
        1,
        [f"serde 1.0.0: yanked (published {BEFORE})"],
    ),
    (
        "too_new_and_yanked",
        release(f"{AFTER}T00:00:00+00:00", yanked=True),
        1,
        ["after the cutoff", "serde 1.0.0: yanked"],
    ),
    ("missing_on_crates_io", None, 1, ["serde 1.0.0: not on crates.io"]),
    (
        "network_error",
        RA.FetchError("crates.io serde 1.0.0: connection refused"),
        1,
        ["serde 1.0.0: network error: crates.io serde 1.0.0: connection refused"],
    ),
]:
    case("release_age", label, partial(ra_check, [P], {("serde", "1.0.0"): answer}), rc, want)
GIT_BP = ("bulletproofs", "5.0.0", chrono("bulletproofs"))
case(
    "release_age",
    "git_source",
    lambda: ra_check([GIT_BP], {("bulletproofs", "5.0.0"): release("2024-01-01T00:00:00+00:00")}),
    0,
    [
        "git source, checked as crates.io bulletproofs 5.0.0",
        "ok bulletproofs 5.0.0 published 2024-01-01",
        "asked: bulletproofs 5.0.0",
    ],
)
case(
    "release_age",
    "git_source_missing_on_crates_io",
    lambda: ra_check([GIT_BP], {}),
    1,
    ["git source, checked as crates.io bulletproofs 5.0.0", "bulletproofs 5.0.0: not on crates.io"],
)
case(
    "release_age",
    "git_source_too_new",
    lambda: ra_check([GIT_BP], {("bulletproofs", "5.0.0"): release(f"{AFTER}T00:00:00+00:00")}),
    1,
    ["bulletproofs 5.0.0: published"],
)
case(
    "release_age",
    "foreign_registry_not_fetched",
    lambda: ra_check([("x", "1.0.0", "registry+https://example.com/index")], {}),
    1,
    ["x 1.0.0: source is not crates.io or git", "asked: (none)"],
)


def changed_case() -> Result:
    base = (
        '[[package]]\nname = "kept"\nversion = "1.0.0"\nsource = "' + CRATES_IO + '"\n\n'
        '[[package]]\nname = "bumped"\nversion = "1.0.0"\nsource = "' + CRATES_IO + '"\n\n'
        '[[package]]\nname = "moved"\nversion = "5.0.0"\nsource = "' + CRATES_IO + '"\n\n'
        '[[package]]\nname = "ferrous-node"\nversion = "0.1.0"\n'
    )
    new = (
        base.replace('"bumped"\nversion = "1.0.0"', '"bumped"\nversion = "1.0.1"')
        .replace(
            f'"moved"\nversion = "5.0.0"\nsource = "{CRATES_IO}"',
            f'"moved"\nversion = "5.0.0"\nsource = "{chrono("moved")}"',
        )
        .replace('"ferrous-node"\nversion = "0.1.0"', '"ferrous-node"\nversion = "0.2.0"')
        + '\n[[package]]\nname = "added"\nversion = "2.0.0"\nsource = "'
        + CRATES_IO
        + '"\n'
    )
    got = [f"{n} {v}" for n, v, _ in RA.changed_packages(base, new)]
    return 0, "changed: " + ", ".join(got) + " |"


case(
    "release_age",
    "changed_packages_new_version_and_source",
    changed_case,
    0,
    ["changed: added 2.0.0, bumped 1.0.1, moved 5.0.0 |"],
)


def throttle_case() -> Result:
    now = [100.0]
    slept: list[float] = []

    def sleep(seconds: float) -> None:
        slept.append(seconds)
        now[0] += seconds

    t = RA.Throttle(lambda: now[0], sleep)
    t.wait()
    now[0] += 0.25
    t.wait()
    now[0] += 3.0
    t.wait()
    return 0, f"slept: {slept}"


case("release_age", "throttle_one_request_per_second", throttle_case, 0, ["slept: [0.75]"])


def request_case() -> Result:
    r = RA.build_request("serde", "1.0.0")
    ua = r.get_header("User-agent")
    return (1 if "@" in str(ua) else 0), f"{r.full_url} ua={ua}"


case(
    "release_age",
    "request_url_and_user_agent",
    request_case,
    0,
    [
        (
            "https://crates.io/api/v1/crates/serde/1.0.0 "
            "ua=ferrous-gate (https://github.com/ChronoCoders/ferrous)"
        )
    ],
)


def fetch_with(opener: Callable[..., Any]) -> Result:
    saved = urllib.request.urlopen
    urllib.request.urlopen = opener
    try:
        result = RA.crates_io_fetch(RA.Throttle(time.monotonic, lambda s: None))("serde", "1.0.0")
        return 0, f"result: {result}"
    except RA.FetchError as e:
        return 1, f"FetchError: {e}"
    finally:
        urllib.request.urlopen = saved


def http_error(code: int) -> Callable[..., Any]:
    def opener(*args: Any, **kwargs: Any) -> Any:
        raise urllib.error.HTTPError("u", code, "status", {}, None)  # type: ignore[arg-type]

    return opener


def url_error(*args: Any, **kwargs: Any) -> Any:
    raise urllib.error.URLError("connection refused")


case(
    "release_age", "fetch_404_is_missing", lambda: fetch_with(http_error(404)), 0, ["result: None"]
)
case(
    "release_age",
    "fetch_http_500_fails_closed",
    lambda: fetch_with(http_error(500)),
    1,
    ["FetchError: crates.io serde 1.0.0: HTTP 500 status"],
)
case(
    "release_age",
    "fetch_network_error_fails_closed",
    lambda: fetch_with(url_error),
    1,
    ["FetchError: crates.io serde 1.0.0: <urlopen error connection refused>"],
)


def main_case(base_lock: str | None, new_lock: str, origin: bool = True) -> Result:
    with tempfile.TemporaryDirectory() as d:
        repo(d, {"Cargo.lock": base_lock} if base_lock is not None else {"x.txt": "x\n"})
        if origin:
            git(d, "update-ref", "refs/remotes/origin/main", "HEAD")
        write(d, "Cargo.lock", new_lock)
        return run(["python3", "-I", RELEASE_AGE], d)


CHANGED_LOCK = append_pkg(REAL_LOCK, "itoa", "1.0.15", CRATES_IO)
case(
    "release_age",
    "main_nothing_changed",
    lambda: main_case(REAL_LOCK, REAL_LOCK),
    0,
    ["release age: no new or changed packages"],
)
case(
    "release_age",
    "main_base_ref_missing",
    lambda: main_case(REAL_LOCK, REAL_LOCK, origin=False),
    1,
    ["no base lockfile at origin/main:Cargo.lock"],
)
case(
    "release_age",
    "main_base_lockfile_missing",
    lambda: main_case(None, REAL_LOCK),
    1,
    ["no base lockfile at origin/main:Cargo.lock"],
)
case(
    "release_age",
    "main_network_error_fails_closed",
    lambda: main_case(REAL_LOCK, CHANGED_LOCK),
    1,
    ["itoa 1.0.15: network error:", "release age: 1 problems in 1 new or changed packages"],
)


def main() -> int:
    start = time.monotonic()
    if not os.path.isdir(ADVISORY_DB):
        print(f"self-test: no advisory database at {ADVISORY_DB}; run cargo audit once to fetch it")
        return 1
    failures: list[str] = []
    for group, name, fn, want_rc, want in CASES:
        try:
            rc, out = fn()
        except Exception as e:  # noqa: BLE001 - a fixture that cannot be built is a failed case
            rc, out = -1, f"fixture error: {type(e).__name__}: {e}"
        problems = []
        if rc != want_rc:
            problems.append(f"exit {rc}, wanted {want_rc}")
        if want is None:
            if out:
                problems.append("output not empty")
        else:
            problems += [f"missing {w!r}" for w in want if w not in out]
        label = f"{group}/{name}"
        if problems:
            first = out.splitlines()[:6]
            failures.append(f"FAIL {label}: {'; '.join(problems)}; output: {first}")
            print(failures[-1])
        else:
            print(f"PASS {label}")
    elapsed = time.monotonic() - start
    for f in failures:
        print(f)
    print(
        f"self-test: {len(CASES) - len(failures)} passed, {len(failures)} failed, "
        f"{len(CASES)} cases in {elapsed:.0f} s"
    )
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
