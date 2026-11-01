# rasm-one

A 68k assembler in Rust using Motorola syntax. It is a library first, with a thin
CLI, and also builds to WASM. See `docs/brief.md` for the goals and the GitHub
issues for progress.

## Using it

Build with `cargo build --release`, which produces the `rasm-one` command:

```
rasm-one <source>... -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-kick1hunks] [-m68000 ... -m68060] [-m68881 | -m68882 | -no-fpu] [-m68851] [-I<dir>]... [-L <listing>] [-linedebug [-noabspath]] [-no-opt | -opt-o+ | -opt-o- | -opt-o<rule>+ | -opt-o<rule>-]...
```

| Option | Output |
|---|---|
| `-Fhunkexe` | Amiga hunk executable (the default) |
| `-kick1hunks` | Make an executable that also loads on Kickstart 1.x. By default, as in vasm, the compact relocations need Kickstart 2.0 or later |
| `-Fhunk` | Amiga hunk object file, for linking |
| `-Fbin` | Raw binary, for bootblocks and trackloaded code |
| `-m68000` ... `-m68060` | Target CPU; 68000 by default |
| `-m68881`, `-m68882` | A floating-point coprocessor alongside a 68020 or 68030 |
| `-m68851` | A 68851 MMU alongside a 68020 or 68030 |
| `-no-fpu` | No floating-point instructions, even on a 68040 or 68060 |
| `-I<dir>` | Also search this directory for `include` and `incbin` files |
| `-L <file>` | Write a listing |

For the library API, including diagnostics, symbols and the line table, run
`cargo doc --open`.

## Status

Integer instructions for the 68000 to 68060, the FPU and the MMU are complete and
compared with vasm. rasm-one optimises the way vasm does by default. `-no-opt` or `Options::unoptimised` turns
that off. Rules can be chosen with `-opt-o-` and `-opt-oN+`, or `opt` in
the source: 1 to 5, 7 to 12, `g` and `f`. vasm's optional rules, which are off by default, aren't
here yet. Output is compared byte for byte with vasm in both modes, apart from rare layouts in
[#56](https://github.com/mwulffn/rasm-one/issues/56). Where vasm has a bug, rasm-one does the
correct thing instead; see `docs/vasm-bugs.md`.

## Debugging

`-linedebug`, or `Options::line_debug`, adds line information to hunk files in SAS/C's `LINE`
format, as vasm does. File names become absolute on the command line unless `-noabspath` is given.
`scripts/emulator-lines.py` sets breakpoints from that information on emulated Amigas and checks
where they stop. It needs Kickstart ROMs, so it runs locally, not in CI.

## Linking

Give the command line several sources to get one executable: `rasm-one a.s b.s -o prog`. In the
library, pass their assemblies to `link`, then `write_hunk_exe`. The linker doesn't read hunk
object files or libraries yet, and it can't make raw binaries. It is compared with vlink by what a
loader would put in memory, using `RASM_VLINK` from `scripts/fetch-vlink.sh`.

## Testing

```
export RASM_VASM=$(scripts/fetch-vasm.sh)
cargo test --workspace
```

The script builds a pinned, unmodified vasm under `target/`, which is never
committed. Without `RASM_VASM`, comparison tests are skipped with a message.

`scripts/fetch-corpus.sh` downloads real-world sources from Aminet for
`tests/corpus.rs`; set `RASM_CORPUS` to the directory it prints.

Two kinds of test run the output. To run executables under
[vamos](https://github.com/cnvogelg/amitools), install it with
`uv tool install 'amitools[vamos]'` and set `RASM_VAMOS` to its path. To boot one
from a floppy on an emulated A500 and A1200, using
[vamiga-lua](https://github.com/mwulffn/vamiga-lua), set `RASM_VAMIGA_LUA`,
`RASM_KICKSTART_13` and `RASM_KICKSTART_31` and run `scripts/emulator-smoke.sh`.
That one is local only, since Kickstart ROMs can't be redistributed. The ROMs must
be plain images: one with a Cloanto header, 11 bytes longer, did not boot.

## Reference material

The instruction table is transcribed from the
[M68000 Family Programmer's Reference Manual](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf).
The list of instructions trapped on the 68060 comes from the MC68060 User's Manual.
Both are third-party copyrighted and not committed; keep local copies at
`docs/M68000PRM.pdf` and `docs/MC68060UM.pdf`.

## Licence

MIT. See `LICENSE`.
