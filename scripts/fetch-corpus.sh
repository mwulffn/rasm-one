#!/bin/sh
# Downloads and unpacks real-world sources for tests/corpus.rs. They belong to their
# authors and are distributed through Aminet, so they are never committed. Prints
# the directory; use it as RASM_CORPUS. Needs lha.
set -eu

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dir=$root/target/corpus
mkdir -p "$dir"

sha256() {
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum "$1" | cut -d' ' -f1
    else
        shasum -a 256 "$1" | cut -d' ' -f1
    fi
}

# usage: fetch <Aminet path without .lha> <directory> <sha256>
fetch() {
    archive=$dir/$2.lha
    if [ -d "$dir/$2" ]; then
        return
    fi
    curl -sSfL --retry 3 -o "$archive" "http://aminet.net/$1.lha"
    actual=$(sha256 "$archive")
    if [ "$actual" != "$3" ]; then
        echo "$1 checksum mismatch: got $actual" >&2
        exit 1
    fi
    mkdir "$dir/$2"
    (cd "$dir/$2" && lha -xq "$archive" >/dev/null)
}

fetch dev/asm/ACCSources acc 5165ad85cf074f2875108495ce6b5edccb179b275316b0372b675a9569d91e6b
fetch dev/asm/8wayscroller scroller d5f4fecefdc87a5d5fa26da032e566bf97ac50e3be4eef1641d9d01aeb92c813
echo "$dir"
