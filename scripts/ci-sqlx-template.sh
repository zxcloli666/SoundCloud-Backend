#!/bin/sh
set -eu

lock=$1/Cargo.lock
out=$RUNNER_TEMP/sqlx-template.toml
: > "$out"

version=$(awk '$0 == "name = \"sqlx-postgres\"" { getline; gsub(/version = |"/, ""); print; exit }' "$lock")
[ -n "$version" ] || exit 0
src=$(ls -d "${CARGO_HOME:-$HOME/.cargo}"/registry/src/*/"sqlx-postgres-$version" 2>/dev/null | head -1)
[ -n "$src" ] || { echo "::warning::sqlx-postgres $version is not fetched, tests migrate every database"; exit 0; }

dest=$RUNNER_TEMP/sqlx-postgres-$version
rm -rf "$dest"
cp -R "$src" "$dest"
if ! patch -s -d "$dest" -p1 < scripts/ci-sqlx-template.patch; then
    echo "::warning::scripts/ci-sqlx-template.patch does not apply to sqlx-postgres $version, tests migrate every database"
    exit 0
fi
find "$dest" -exec touch -h -t 200001010000 {} +
printf '[patch.crates-io]\nsqlx-postgres = { path = "%s" }\n' "$dest" > "$out"
echo "sqlx-postgres $version: databases of migrated tests are cloned from a template"
