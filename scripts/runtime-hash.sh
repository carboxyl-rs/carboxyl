#!/usr/bin/env bash

export CARBONYL_ROOT=$(cd $(dirname -- "$0") && dirname -- $(pwd))

cd "$CARBONYL_ROOT"
source "scripts/env.sh"

while IFS= read -r file; do
    file_sha=$(cat "$file" | openssl sha256)
    sha+="${file_sha: -64} ${file}"$'\n'
done < <(
    {
        printf '%s\n' Cargo.toml Cargo.lock build.rs readme.md
        find src scripts -type f
    } | sort
)

hash=$(echo "$sha" | sort | openssl sha256)

echo -n "${hash: -16}"
