#!/bin/sh
# Assembles tests/amiga/smoke.s, boots it from a floppy on emulated Amigas, and
# reads its result back from emulated memory. Kickstart 1.3 gets the format for
# 1.x, and 3.1 gets the default format.
#
# Local only, because it needs Kickstart ROMs, which can't be redistributed:
#   RASM_KICKSTART_13  Kickstart 1.3 ROM
#   RASM_KICKSTART_31  Kickstart 3.1 ROM for the A1200
#   RASM_VAMIGA_LUA    checkout of vamiga-lua, already built
# Also needs xdftool from amitools.
set -eu

: "${RASM_KICKSTART_13:?set RASM_KICKSTART_13 to a Kickstart 1.3 ROM}"
: "${RASM_KICKSTART_31:?set RASM_KICKSTART_31 to an A1200 Kickstart 3.1 ROM}"
: "${RASM_VAMIGA_LUA:?set RASM_VAMIGA_LUA to a built vamiga-lua checkout}"
PORT=${RASM_EMULATOR_PORT:-5617}
EXPECTED=0000D0DE

root=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
emulator_pid=
cleanup() {
    if [ -n "$emulator_pid" ]; then
        kill "$emulator_pid" 2>/dev/null || true
    fi
    rm -rf "$work"
}
trap cleanup EXIT

client() {
    python3 "$RASM_VAMIGA_LUA/scripts/amiga_lua.py" --port "$PORT" "$1"
}

# usage: boot_and_check <model> <rom> [assembler options]
boot_and_check() {
    model=$1
    rom=$2
    shift 2
    disk=$work/$model.adf
    (cd "$root" && cargo run -q -p rasm-one-cli -- tests/amiga/smoke.s -o "$work/smoke" "$@")
    (
        cd "$work"
        printf 'smoke\n' > startup-sequence
        xdftool "$disk" create + format TEST ofs + boot install + makedir s >/dev/null
        xdftool "$disk" write smoke smoke + write startup-sequence s/startup-sequence >/dev/null
    )

    "$RASM_VAMIGA_LUA/build/vamiga-lua" --model "$model" --rom "$rom" \
        --df0 "$disk" --port "$PORT" --warp > "$work/emulator.log" 2>&1 &
    emulator_pid=$!

    tries=0
    until client 'return 1' >/dev/null 2>&1; do
        tries=$((tries + 1))
        if [ "$tries" -gt 50 ]; then
            echo "emulator did not start:" >&2
            cat "$work/emulator.log" >&2
            exit 1
        fi
        sleep 0.2
    done

    # At most 60 emulated seconds, far longer than booting takes.
    actual=$(client '
        for _ = 1, 60 do
            emu.wait_frames(50)
            if mem.peek_u32(0x104) == 0x5241534D then
                return string.format("%08X", mem.peek_u32(mem.peek_u32(0x100)))
            end
        end
        return "no result"
    ')
    client 'emu.quit()' >/dev/null 2>&1 || true
    wait "$emulator_pid" 2>/dev/null || true
    emulator_pid=

    if [ "$actual" != "$EXPECTED" ]; then
        echo "emulator smoke test failed on $model: expected $EXPECTED, got $actual" >&2
        exit 1
    fi
    echo "emulator smoke test passed on $model: the program computed $actual"
}

boot_and_check A500 "$RASM_KICKSTART_13" -kick1hunks
boot_and_check A1200 "$RASM_KICKSTART_31"
