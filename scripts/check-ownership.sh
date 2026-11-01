#!/usr/bin/env bash
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
python3 -m unittest discover -q -s ops/replication/tests
python3 ops/replication/gen.py --check
python3 ops/replication/check_ownership.py "$@"
