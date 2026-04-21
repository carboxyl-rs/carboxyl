#!/usr/bin/env bash

export CARBONYL_ROOT=$(cd $(dirname -- "$0") && dirname -- "$(pwd)")

source "$CARBONYL_ROOT/scripts/env.sh"

if [ -x "$CARBONYL_ROOT/target/release/carbonyl" ]; then
    "$CARBONYL_ROOT/target/release/carbonyl" "$@"
else
    cargo run --release --bin carbonyl -- "$@"
fi
