# rasm-one

A 68k assembler in Rust using Motorola syntax. It is a library first, with a thin
CLI, and also builds to WASM. Early development: see `docs/brief.md` for the goals
and the GitHub issues for progress.

## Reference material

The instruction table is transcribed from the
[M68000 Family Programmer's Reference Manual](https://www.nxp.com/docs/en/reference-manual/M68000PRM.pdf).
The list of instructions trapped on the 68060 comes from the MC68060 User's Manual.
Both are third-party copyrighted and not committed; keep local copies at
`docs/M68000PRM.pdf` and `docs/MC68060UM.pdf`.

## Licence

MIT. See `LICENSE`.
