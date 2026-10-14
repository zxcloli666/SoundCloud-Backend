#!/bin/sh
set -eu

out="$1"
shift

target_roots() {
    for file in src/lib.rs src/main.rs build.rs; do
        [ -f "$1/$file" ] && echo "$file"
    done
    if [ -d "$1/src/bin" ]; then
        (cd "$1" && find src/bin -maxdepth 2 \( -path 'src/bin/*.rs' -o -path 'src/bin/*/main.rs' \))
    fi
    sed -nE 's/^path *= *"([^"]+)".*/\1/p' "$1/Cargo.toml"
}

find "$@" -name Cargo.toml -not -path '*/target/*' | while read -r manifest; do
    crate=$(dirname "$manifest")
    mkdir -p "$out/$crate"
    cp "$manifest" "$out/$crate/"
    if [ -f "$crate/Cargo.lock" ]; then
        cp "$crate/Cargo.lock" "$out/$crate/"
    fi
    target_roots "$crate" | while read -r root; do
        mkdir -p "$(dirname "$out/$crate/$root")"
        echo 'fn main() {}' > "$out/$crate/$root"
    done
done
