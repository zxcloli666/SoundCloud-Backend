#!/bin/sh
set -eu

rustup toolchain install
active=$(rustup show active-toolchain | cut -d' ' -f1)
rustup toolchain list | cut -d' ' -f1 | grep -vxF "$active" | xargs -r -n1 rustup toolchain uninstall
