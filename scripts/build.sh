#!/usr/bin/env bash

set -eo pipefail

export CARBONYL_ROOT=$(cd $(dirname -- "$0") && dirname -- "$(pwd)")

cd "$CARBONYL_ROOT"
source scripts/env.sh

cargo build --release --bin carbonyl "$@"
