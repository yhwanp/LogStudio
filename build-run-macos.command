#!/bin/sh
# Explicit developer build; the ordinary start scripts remain build-free.
script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd) || exit 1
exec python3 "$script_dir/desktop/build_run_macos.py" "$@"
