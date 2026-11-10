#!/usr/bin/env bash
# Runs the gate self-test. unshare gives it a network namespace with no network, so any network
# call fails and the run is offline by construction. -B keeps Python from writing bytecode next
# to release_age.py, so no stale cache can outlive a source change.
cd "$(git rev-parse --show-toplevel)" || exit 2
exec unshare -rn python3 -I -B scripts/tests/selftest.py "$PWD"
