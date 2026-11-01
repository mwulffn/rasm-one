#!/bin/sh
# Builds the JavaScript bindings and runs them under Node, to prove the WASM
# build really works rather than merely compiles. Needs Node on the PATH.
set -eu

# Must match the wasm-bindgen dependency of crates/rasm-one-wasm.
VERSION=0.2.130

case "$(uname -s)-$(uname -m)" in
    Darwin-arm64)
        target=aarch64-apple-darwin
        sha256=b90ddb9357f584b1af897d58ad77e4cce2b48fc17e2a967bc249a87f72809444
        ;;
    Linux-x86_64)
        target=x86_64-unknown-linux-musl
        sha256=9687f3bb551b9f66f37ed32a40d7913a00f7962db653054e56857bc306c3227f
        ;;
    *)
        echo "no pinned wasm-bindgen tool for this platform" >&2
        exit 1
        ;;
esac

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
dir=$root/target/wasm-bindgen
tool=$dir/wasm-bindgen-$VERSION-$target/wasm-bindgen
if [ ! -x "$tool" ]; then
    mkdir -p "$dir"
    archive=$dir/wasm-bindgen-$VERSION-$target.tar.gz
    url=https://github.com/wasm-bindgen/wasm-bindgen/releases/download/$VERSION
    curl -sSfL -o "$archive" "$url/wasm-bindgen-$VERSION-$target.tar.gz"
    if command -v sha256sum >/dev/null 2>&1; then
        actual=$(sha256sum "$archive" | cut -d' ' -f1)
    else
        actual=$(shasum -a 256 "$archive" | cut -d' ' -f1)
    fi
    if [ "$actual" != "$sha256" ]; then
        echo "wasm-bindgen checksum mismatch: got $actual" >&2
        exit 1
    fi
    tar -xzf "$archive" -C "$dir"
fi

cd "$root"
cargo build --release -p rasm-one-wasm --target wasm32-unknown-unknown
"$tool" --target nodejs --out-dir target/wasm-pkg \
    target/wasm32-unknown-unknown/release/rasm_one_wasm.wasm
ls -l target/wasm-pkg/rasm_one_wasm_bg.wasm
node scripts/wasm-smoke.cjs "$root/target/wasm-pkg/rasm_one_wasm.js"
