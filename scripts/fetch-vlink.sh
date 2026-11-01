#!/bin/sh
# Downloads and builds the pinned, unmodified vlink used as the linker's test reference.
# Like vasm, it is never committed. Prints the path of the binary; use it as RASM_VLINK.
set -eu

URL=http://phoenix.owl.de/tags/vlink0_17a.tar.gz
SHA256=f6754913d47bc97cf4771cc0aa7c51de368a04894be35b8dc5e4beac527f5b82

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dir=$root/target/vlink
bin=$dir/vlink/vlink

if [ ! -x "$bin" ]; then
    mkdir -p "$dir"
    curl -sSfL -o "$dir/vlink.tar.gz" "$URL"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$dir/vlink.tar.gz" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$dir/vlink.tar.gz" | cut -d' ' -f1)
    fi
    if [ "$actual" != "$SHA256" ]; then
        echo "vlink checksum mismatch: got $actual" >&2
        exit 1
    fi
    rm -rf "$dir/vlink"
    tar -xzf "$dir/vlink.tar.gz" -C "$dir"
    make -C "$dir/vlink" >/dev/null 2>&1
fi
echo "$bin"
