# rasm-one

A 68k assembler in Rust using Motorola syntax. It is a library first, with a thin
CLI, and also builds to WASM. See `docs/brief.md` for the goals and the GitHub
issues for progress.

## Using it

Build with `cargo build --release`, which produces the `rasm-one` command:

```
rasm-one <source> -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-kick1hunks] [-m68000 ... -m68060] [-m68881 | -m68882 | -no-fpu] [-I<dir>]... [-L <listing>]
```

| Option | Output |
|---|---|
| `-Fhunkexe` | Amiga hunk executable (the default) |
| `-kick1hunks` | Make an executable that also loads on Kickstart 1.x. By default, as in vasm, the compact relocations need Kickstart 2.0 or later |
| `-Fhunk` | Amiga hunk object file, for linking |
| `-Fbin` | Raw binary, for bootblocks and trackloaded code |
| `-m68000` ... `-m68060` | Target CPU; 68000 by default |
| `-m68881`, `-m68882` | A floating-point coprocessor alongside a 68020 or 68030 |
| `-no-fpu` | No floating-point instructions, even on a 68040 or 68060 |
| `-I<dir>` | Also search this directory for `include` and `incbin` files |
| `-L <file>` | Write a listing |

For the library API, including diagnostics, symbols and the line table, run
`cargo doc --open`.

## Status

Integer instructions for the 68000 to 68060 and the FPU are complete and compared
with vasm. Output is compared byte for byte against `vasm -no-opt`, so nothing is
optimised yet. Where vasm has a bug, rasm-one does the correct thing instead; see
`docs/vasm-bugs.md`. The MMU is not done yet; see the issues.

## Testing

```
export RASM_VASM=$(scripts/fetch-vasm.sh)
cargo test --workspace
```

The script builds a pinned, unmodified vasm under `target/`, which is never
committed. Without `RASM_VASM`, comparison tests are skipped with a message.

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
