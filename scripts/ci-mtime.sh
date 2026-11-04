#!/bin/sh
set -eu

stamp=.ci-sha

if [ "${1:-}" = stamp ]; then
    git rev-parse HEAD > "$stamp"
    exit 0
fi

old=$(cat "$stamp" 2>/dev/null || true)
[ -n "$old" ] || exit 0
git cat-file -e "$old^{commit}" 2>/dev/null || git fetch --quiet --no-tags --depth=1 origin "$old" 2>/dev/null || exit 0

git ls-files -z | xargs -0 touch -c -t 200001010000
git diff --name-only --no-renames -z "$old" HEAD | xargs -0 -r touch -c
echo "source times from $(echo "$old" | cut -c1-7): $(git diff --name-only --no-renames "$old" HEAD | wc -l | tr -d ' ') files changed"

if git diff --quiet "$old" HEAD -- api/migrations api/migrations-ops; then
    exit 0
fi
git grep -l '^sqlx = ' -- '*Cargo.toml' | while read -r manifest; do
    touch -c "$(dirname "$manifest")/src/lib.rs" "$(dirname "$manifest")/src/main.rs"
done
echo "schema changed: crates with sqlx macros rebuild"
