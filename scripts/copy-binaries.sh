#!/usr/bin/env bash

export CARBONYL_ROOT=$(cd $(dirname -- "$0") && dirname -- $(pwd))

cd "$CARBONYL_ROOT"
source "scripts/env.sh"

triple="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
dest="build/pre-built/$triple"
src="target/$triple/release/carbonyl"

if [ ! -f "$src" ]; then
    src="target/release/carbonyl"
fi

if [ ! -f "$src" ]; then
    echo "No built carbonyl binary found. Run ./scripts/build.sh first."
    exit 1
fi

rm -rf "$dest"
mkdir -p "$dest"

cp "$src" "$dest/carbonyl"

echo "Binary copied to $dest"
