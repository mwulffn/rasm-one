#!/usr/bin/env python3
"""Sets breakpoints from a program's line debug information, and checks where it stops.

Links tests/amiga/lines_main.s and lines_other.s with -linedebug, boots them from a floppy on
emulated Amigas, and reads the LINE tables from the file itself. Every line with "moveq #N,d7"
must have that instruction at its address in memory, and running must stop on those lines in
order, with d7 holding the previous N.

Local only, because it needs Kickstart ROMs, which can't be redistributed. It takes the same
settings as scripts/emulator-smoke.sh: RASM_KICKSTART_13, RASM_KICKSTART_31 and RASM_VAMIGA_LUA.
Also needs xdftool from amitools.
"""

import os
import re
import shutil
import socket
import struct
import subprocess
import sys
import tempfile
import time
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
SOURCES = ROOT / "tests" / "amiga"
PORT = int(os.environ.get("RASM_EMULATOR_PORT", "5618"))
EXPECTED_ORDER = [1, 2, 3, 4, 5, 4, 5, 6, 7, 8]
MARKER = re.compile(r"moveq\s+#(\d+),d7")
HUNK_NAMES = {0x3E7, 0x3E8}
CONTENTS = {0x3E9, 0x3EA}


def line_tables(data: bytes) -> list[list[tuple[str, int, int]]]:
    """Returns (file, line, offset in hunk) for each hunk of an executable."""
    at = 0

    def long() -> int:
        nonlocal at
        value = struct.unpack_from(">I", data, at)[0]
        at += 4
        return value

    assert long() == 0x3F3
    assert long() == 0
    count = long()
    at += 8 + count * 4
    hunks: list[list[tuple[str, int, int]]] = [[] for _ in range(count)]
    current = 0
    while at < len(data):
        kind = long() & 0x3FFFFFFF
        if kind in CONTENTS or kind in HUNK_NAMES:
            length = long()
            at += length * 4
        elif kind == 0x3EB:
            long()
        elif kind in {0x3EC}:
            while (offsets := long()) != 0:
                at += 4 + offsets * 4
        elif kind == 0x3F7:
            while True:
                offsets = struct.unpack_from(">H", data, at)[0]
                at += 2
                if offsets == 0:
                    break
                at += 2 + offsets * 2
            at += at % 4
        elif kind == 0x3F0:
            while (length := long()) != 0:
                at += (length & 0xFFFFFF) * 4 + 4
        elif kind == 0x3F1:
            end = long() * 4 + at
            base = long()
            assert data[at : at + 4] == b"LINE"
            at += 4
            name_longs = long()
            name = data[at : at + name_longs * 4].rstrip(b"\0").decode()
            at += name_longs * 4
            while at < end:
                line = long()
                hunks[current].append((name, line, base + long()))
        elif kind == 0x3F2:
            current += 1
        else:
            raise ValueError(f"unexpected hunk {kind:#x}")
    return hunks


def marked_lines(
    tables: list[list[tuple[str, int, int]]],
) -> list[tuple[int, int, int]]:
    """Returns (hunk, offset, N) for table entries on a line with a marker."""
    marked = []
    for hunk, table in enumerate(tables):
        for name, line, offset in table:
            text = (SOURCES / Path(name).name).read_text().splitlines()[line - 1]
            found = MARKER.search(text)
            if found:
                marked.append((hunk, offset, int(found[1])))
    return marked


class Emulator:
    """A vamiga-lua process, driven over its socket."""

    def __init__(self, checkout: Path, model: str, rom: str, disk: Path) -> None:
        program = checkout / "build" / "vamiga-lua"
        arguments = [program, "--model", model, "--rom", rom, "--df0", disk]
        arguments += ["--port", str(PORT), "--warp"]
        self.process = subprocess.Popen(
            arguments, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
        )
        self.client = [sys.executable, checkout / "scripts" / "amiga_lua.py"]
        self.client += ["--port", str(PORT)]
        for _ in range(50):
            with socket.socket() as probe:
                if probe.connect_ex(("127.0.0.1", PORT)) == 0:
                    return
            time.sleep(0.2)
        raise RuntimeError("the emulator did not start")

    def run(self, code: str) -> str:
        done = subprocess.run(
            [*self.client, code], capture_output=True, text=True, check=False
        )
        if done.returncode != 0:
            raise RuntimeError(f"Lua failed: {done.stderr}{done.stdout}")
        return done.stdout.strip()

    def close(self) -> None:
        self.process.terminate()
        self.process.wait()


def wait_for_mark(emulator: Emulator) -> None:
    """Waits for the program to be running, for at most 60 emulated seconds."""
    state = emulator.run(
        """
        for _ = 1, 60 do
            emu.wait_frames(50)
            if mem.peek_u32(0x104) == 0x5241534D then return "ready" end
        end
        return "no mark"
        """
    )
    if state != "ready":
        raise RuntimeError("the program never started")


def check_model(
    model: str, rom: str, flags: list[str], checkout: Path, work: Path
) -> list[str]:
    """Runs the program on one Amiga, and returns what went wrong."""
    program = work / "lines"
    subprocess.run(
        ["cargo", "run", "-q", "-p", "rasm-one-cli", "--"]
        + ["tests/amiga/lines_main.s", "tests/amiga/lines_other.s"]
        + ["-Itests/amiga", "-linedebug", "-noabspath", "-o", str(program), *flags],
        cwd=ROOT,
        check=True,
    )
    disk = work / f"{model}.adf"
    (work / "startup-sequence").write_text("lines\n")
    subprocess.run(
        [
            "xdftool",
            disk,
            "create",
            "+",
            "format",
            "TEST",
            "ofs",
            "+",
            "boot",
            "install",
        ]
        + ["+", "makedir", "s"],
        cwd=work,
        check=True,
        stdout=subprocess.DEVNULL,
    )
    subprocess.run(
        ["xdftool", disk, "write", "lines", "lines", "+"]
        + ["write", "startup-sequence", "s/startup-sequence"],
        cwd=work,
        check=True,
        stdout=subprocess.DEVNULL,
    )

    marked = marked_lines(line_tables(program.read_bytes()))
    found = sorted({number for _, _, number in marked})
    problems = []
    if found != sorted(set(EXPECTED_ORDER)):
        problems.append(f"the line tables mark {found}")

    emulator = Emulator(checkout, model, rom, disk)
    try:
        wait_for_mark(emulator)
        # Hunks are placed by the system, so ask the program where. In this program, hunk 0
        # is at $100's address, and the far hunk, which is the last, at $108's.
        last = max(hunk for hunk, _, _ in marked)
        entries = ", ".join(
            f"{{{hunk},{offset},{number}}}" for hunk, offset, number in marked
        )
        result = emulator.run(
            f"""
            local bases = {{[0] = mem.peek_u32(0x100), [{last}] = mem.peek_u32(0x108)}}
            local bad = {{}}
            hits = {{}}
            for _, entry in ipairs({{{entries}}}) do
                local address = bases[entry[1]] + entry[2]
                local word = mem.peek_u16(address)
                if word ~= 0x7E00 + entry[3] then
                    bad[#bad + 1] = string.format("%d: %04X at %08X", entry[3], word, address)
                end
                dbg.bpset(address, function()
                    hits[#hits + 1] = string.format("%d:%d", entry[3], cpu.d7)
                end)
            end
            mem.poke_u32(0x10C, 1)
            for _ = 1, 20 do
                emu.wait_frames(10)
                if mem.peek_u32(0x110) == 0x444F4E45 then
                    return table.concat(bad, ";") .. "|" .. table.concat(hits, ",")
                end
            end
            return table.concat(bad, ";") .. "|never finished: " .. table.concat(hits, ",")
            """
        )
    finally:
        try:
            emulator.run("emu.quit()")
        except RuntimeError:
            pass
        emulator.close()

    wrong_memory, hits = result.split("|", 1)
    if wrong_memory:
        problems.append(f"wrong instruction at a line's address: {wrong_memory}")
    actual = hits.split(",")
    # d7 is unknown before the first stop.
    actual_order = [hit.split(":") for hit in actual if ":" in hit]
    if [hit[0] for hit in actual_order] != [str(n) for n in EXPECTED_ORDER]:
        problems.append(f"stopped in the wrong order: {hits}")
    elif [hit[1] for hit in actual_order[1:]] != [str(n) for n in EXPECTED_ORDER[:-1]]:
        problems.append(f"d7 was wrong at a stop: {hits}")
    return problems


def main() -> int:
    missing = [
        name
        for name in ("RASM_KICKSTART_13", "RASM_KICKSTART_31", "RASM_VAMIGA_LUA")
        if name not in os.environ
    ]
    if missing:
        print(f"set {', '.join(missing)}", file=sys.stderr)
        return 2
    checkout = Path(os.environ["RASM_VAMIGA_LUA"]).expanduser()
    work = Path(tempfile.mkdtemp())
    failed = False
    try:
        for model, rom, flags in (
            ("A500", os.environ["RASM_KICKSTART_13"], ["-kick1hunks"]),
            ("A1200", os.environ["RASM_KICKSTART_31"], []),
        ):
            problems = check_model(model, rom, flags, checkout, work)
            for problem in problems:
                print(f"{model}: {problem}", file=sys.stderr)
            failed = failed or bool(problems)
            if not problems:
                print(f"line debug check passed on {model}")
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
