#!/bin/sh
set -eu

case "${EVENT:-}" in
pull_request) base=$(git rev-parse HEAD^1) ;;
push) base=${BEFORE:-} ;;
*) base= ;;
esac

case "$base" in
'' | 0000000000000000000000000000000000000000) base= ;;
*) git cat-file -e "$base^{commit}" 2>/dev/null || git fetch --quiet --no-tags --depth=1 origin "$base" 2>/dev/null || base= ;;
esac

echo "changes since ${base:-no base, everything runs}" >&2

shared='.github/workflows/ci.yml rust-toolchain.toml .cargo scripts/ci-changes.sh scripts/ci-mtime.sh scripts/ci-toolchain.sh'

emit() {
    name=$1
    shift
    files=all
    if [ -n "$base" ]; then
        files=$(git diff --name-only --no-renames "$base" HEAD -- $shared "$@" ':!*.md' ':!*.iml' ':!*.example')
    fi
    if [ -n "$files" ]; then echo "$name=true"; else echo "$name=false"; fi
}

emit api api backend-contracts utils
emit jobs jobs backend-contracts utils api/migrations api/migrations-ops api/queries
emit services streaming storage gateway utils
emit prod api jobs streaming backend-contracts utils
emit yanked '*Cargo.lock' deny.toml
emit worker worker backend-contracts/contract backend-contracts/fixtures
