#!/usr/bin/env bash
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
psql_cmd="${PSQL:-psql}"
out="${SCHEMA_OUT:-$here/schema.json}"

dump() {
  $psql_cmd -XAtq -v ON_ERROR_STOP=1 "$1" < "$here/schema.sql"
}

{
  printf '{"core": '
  dump "$1"
  printf ', "ops": '
  dump "$2"
  printf '}\n'
} | python3 "$here/schema_json.py" > "$out"
