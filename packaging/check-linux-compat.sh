#!/usr/bin/env bash
# Verify release ELF binaries stay within the documented glibc baseline and do
# not regain OpenSSL runtime dependencies. Usage: script BIN... MAX_GLIBC
set -euo pipefail

if [ "$#" -lt 2 ]; then
    echo "Usage: $0 BINARY... MAX_GLIBC" >&2
    exit 2
fi

max_glibc="${!#}"
set -- "${@:1:$(($# - 1))}"

version_gt() {
    [ "$(printf '%s\n%s\n' "$1" "$2" | sort -V | tail -n1)" = "$1" ] && [ "$1" != "$2" ]
}

for binary in "$@"; do
    [ -x "$binary" ] || { echo "Not an executable: $binary" >&2; exit 1; }

    required="$({ readelf --version-info "$binary" || true; } \
        | sed -n 's/.*GLIBC_\([0-9][0-9.]*\).*/\1/p' \
        | sort -Vu | tail -n1)"
    if [ -n "$required" ] && version_gt "$required" "$max_glibc"; then
        echo "$binary requires glibc $required (maximum supported: $max_glibc)" >&2
        exit 1
    fi

    if readelf -d "$binary" | grep -Eq 'NEEDED.*(libssl|libcrypto)'; then
        echo "$binary has a direct OpenSSL runtime dependency" >&2
        exit 1
    fi

    echo "OK: $binary (glibc ${required:-none}, no direct OpenSSL runtime dependency)"
done
