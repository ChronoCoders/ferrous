"""Release-age check for Cargo.lock against the lockfile on origin/main.

Every package whose name, version or source is new against the base lockfile must be a crates.io
release published on or before CUTOFF and not yanked. A git-sourced package is checked as the
crates.io release with the same name and version, which must exist. Any network or HTTP error
fails the check. Run from the repository root.
"""

import datetime
import json
import subprocess
import sys
import time
import tomllib
import urllib.error
import urllib.request
from collections.abc import Callable
from typing import Any

CUTOFF = datetime.date(2026, 11, 2)
CRATES_IO = "registry+https://github.com/rust-lang/crates.io-index"
API = "https://crates.io/api/v1/crates"
USER_AGENT = "ferrous-gate (https://github.com/ChronoCoders/ferrous)"
MIN_INTERVAL = 1.0

Package = tuple[str, str, str]
Fetch = Callable[[str, str], dict[str, Any] | None]


class FetchError(Exception):
    pass


class BaseError(Exception):
    pass


def packages(lock_text: str) -> set[Package]:
    lock = tomllib.loads(lock_text)
    return {
        (p["name"], p["version"], p["source"]) for p in lock.get("package", []) if "source" in p
    }


def changed_packages(base_text: str, new_text: str) -> list[Package]:
    return sorted(packages(new_text) - packages(base_text))


def build_request(name: str, version: str) -> urllib.request.Request:
    return urllib.request.Request(f"{API}/{name}/{version}", headers={"User-Agent": USER_AGENT})


class Throttle:
    def __init__(self, clock: Callable[[], float], sleep: Callable[[float], None]) -> None:
        self.clock = clock
        self.sleep = sleep
        self.last: float | None = None

    def wait(self) -> None:
        if self.last is not None:
            remaining = MIN_INTERVAL - (self.clock() - self.last)
            if remaining > 0:
                self.sleep(remaining)
        self.last = self.clock()


def crates_io_fetch(throttle: Throttle) -> Fetch:
    def fetch(name: str, version: str) -> dict[str, Any] | None:
        throttle.wait()
        try:
            with urllib.request.urlopen(build_request(name, version), timeout=30) as response:
                body: dict[str, Any] = json.load(response)
        except urllib.error.HTTPError as e:
            if e.code == 404:
                return None
            raise FetchError(f"crates.io {name} {version}: HTTP {e.code} {e.reason}") from e
        except (urllib.error.URLError, OSError, ValueError) as e:
            raise FetchError(f"crates.io {name} {version}: {e}") from e
        return body

    return fetch


def check(
    changed: list[Package], fetch: Fetch, cutoff: datetime.date
) -> tuple[list[str], list[str]]:
    lines: list[str] = []
    problems: list[str] = []
    for name, version, source in changed:
        if source.startswith("git+"):
            lines.append(f"git source, checked as crates.io {name} {version}")
        elif source != CRATES_IO:
            problems.append(f"{name} {version}: source is not crates.io or git: {source}")
            continue
        try:
            body = fetch(name, version)
        except FetchError as e:
            problems.append(f"{name} {version}: network error: {e}")
            continue
        if body is None:
            problems.append(f"{name} {version}: not on crates.io")
            continue
        release = body["version"]
        published = datetime.datetime.fromisoformat(release["created_at"])
        day = published.astimezone(datetime.UTC).date()
        if day > cutoff:
            problems.append(f"{name} {version}: published {day}, after the cutoff {cutoff}")
        if release["yanked"]:
            problems.append(f"{name} {version}: yanked (published {day})")
        if day <= cutoff and not release["yanked"]:
            lines.append(f"ok {name} {version} published {day}")
    return lines, problems


def base_lockfile() -> str:
    run = subprocess.run(
        ["git", "show", "origin/main:Cargo.lock"], check=False, capture_output=True, text=True
    )
    if run.returncode != 0:
        raise BaseError(f"no base lockfile at origin/main:Cargo.lock: {run.stderr.strip()}")
    return run.stdout


def main() -> int:
    try:
        base = base_lockfile()
    except BaseError as e:
        print(e)
        return 1
    with open("Cargo.lock") as f:
        changed = changed_packages(base, f.read())
    if not changed:
        print("release age: no new or changed packages")
        return 0
    lines, problems = check(changed, crates_io_fetch(Throttle(time.monotonic, time.sleep)), CUTOFF)
    for line in lines + problems:
        print(line)
    if problems:
        print(f"release age: {len(problems)} problems in {len(changed)} new or changed packages")
        return 1
    print(f"release age: {len(changed)} new or changed packages, all on or before {CUTOFF}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
