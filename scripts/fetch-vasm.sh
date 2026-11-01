#!/bin/sh
# Downloads and builds the pinned, unmodified vasm used as the test reference.
# vasm's licence forbids redistributing modified versions, so it is never
# committed. Prints the path of the binary; use it as RASM_VASM.
set -eu

URL=http://phoenix.owl.de/tags/vasm2_0d.tar.gz
SHA256=a818b0cef018c3b88be998de2c15ece83303cbe4a15f0b391c0203710b262f2e

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dir=$root/target/vasm
bin=$dir/vasm/vasmm68k_mot

if [ ! -x "$bin" ]; then
    mkdir -p "$dir"
    curl -sSfL -o "$dir/vasm.tar.gz" "$URL"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$dir/vasm.tar.gz" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$dir/vasm.tar.gz" | cut -d' ' -f1)
    fi
    if [ "$actual" != "$SHA256" ]; then
        echo "vasm checksum mismatch: got $actual" >&2
        exit 1
    fi
    rm -rf "$dir/vasm"
    tar -xzf "$dir/vasm.tar.gz" -C "$dir"
    make -C "$dir/vasm" CPU=m68k SYNTAX=mot >/dev/null 2>&1
fi
echo "$bin"
