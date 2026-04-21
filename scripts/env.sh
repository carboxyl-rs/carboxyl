#!/usr/bin/env bash

set -eo pipefail

if [ -z "$CARBONYL_ROOT" ]; then
    echo "CARBONYL_ROOT should be defined"

    exit 2
fi
