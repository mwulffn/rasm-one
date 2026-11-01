# rasm-one

A 68k assembler in Rust using Motorola syntax. It is a library first, with a thin
CLI, and also builds to WASM. See `docs/brief.md` for the goals and the GitHub
issues for progress.

## Using it

Build with `cargo build --release`, which produces the `rasm-one` command:

```
rasm-one <source> -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-I<dir>]... [-L <listing>]
```

| Option | Output |
|---|---|
| `-Fhunkexe` | Amiga hunk executable (the default) |
| `-Fhunk` | Amiga hunk object file, for linking |
| `-Fbin` | Raw binary, for bootblocks and trackloaded code |
| `-I<dir>` | Also search this directory for `include` and `incbin` files |
| `-L <file>` | Write a listing |

For the library API, including diagnostics, symbols and the line table, run
`cargo doc --open`.

## Status

The 68000 instruction set is complete and compared with vasm. Output is compared
byte for byte against `vasm -no-opt`, so nothing is optimised yet. Other CPUs,
the FPU and the MMU are not yet checked; see the issues.

## Testing

```
export RASM_VASM=$(scripts/fetch-vasm.sh)
cargo test --workspace
```

The script builds a pinned, unmodified vasm under `target/`, which is never
committed. Without `RASM_VASM`, comparison tests are skipped with a message.

## Reference material

The instruction table is transcribed from the
[M68000 Family Programmer's Reference Manual](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf).
The list of instructions trapped on the 68060 comes from the MC68060 User's Manual.
Both are third-party copyrighted and not committed; keep local copies at
`docs/M68000PRM.pdf` and `docs/MC68060UM.pdf`.

## Licence

MIT. See `LICENSE`.
