#!/bin/sh
set -eu

out=$1
app=$2
shift 2

own_git=https://github.com/zxcloli666/SoundCloud-Backend

target_roots() {
    for file in src/lib.rs src/main.rs build.rs; do
        if [ -f "$1/$file" ]; then echo "$file"; fi
    done
    if [ -d "$1/src/bin" ]; then
        (cd "$1" && find src/bin -maxdepth 2 \( -path 'src/bin/*.rs' -o -path 'src/bin/*/main.rs' \))
    fi
    sed -nE 's/^path *= *"([^"]+)".*/\1/p' "$1/Cargo.toml"
}

normalize_manifest() {
    awk -v root="$2" -v git="$own_git" '
        /^\[/ { section = $0 }
        (section == "[package]" || section == "[workspace.package]") && /^version *= *"/ {
            print "version = \"0.0.0\""
            next
        }
        /path *= *"/ {
            gsub(/version *= *"[^"]*" *, */, "")
            gsub(/, *version *= *"[^"]*"/, "")
        }
        index($0, git) && match($0, /^[A-Za-z0-9_-]+/) {
            name = substr($0, RSTART, RLENGTH)
            print name " = { path = \"" root "/utils/" name "\" }"
            next
        }
        { print }
    ' "$1"
}

normalize_lock() {
    awk '
        BEGIN { RS = ""; ORS = "\n\n" }
        /^\[\[package\]\]/ && !/\nsource = / { sub(/\nversion = "[^"]*"/, "\nversion = \"0.0.0\"") }
        { print }
    ' "$1"
}

find "$app" "$@" -name Cargo.toml -not -path '*/target/*' | while read -r manifest; do
    crate=$(dirname "$manifest")
    mkdir -p "$out/$crate"
    normalize_manifest "$manifest" "$(echo "$crate" | sed -E 's|[^/]+|..|g')" > "$out/$manifest"
    target_roots "$crate" | while read -r file; do
        mkdir -p "$(dirname "$out/$crate/$file")"
        echo 'fn main() {}' > "$out/$crate/$file"
    done
done

normalize_lock "$app/Cargo.lock" > "$out/$app/Cargo.lock"
