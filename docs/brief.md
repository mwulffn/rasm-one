rasm-one: a 68k assembler in Rust (Motorola syntax)

Shape
- Library crate first, thin CLI crate on top. Must also build to WASM
  (it will run in a browser IDE later).
- No direct filesystem access in the library: sources, include and
  incbin go through a file-provider trait (native fs for the CLI,
  in-memory/OPFS for the browser).
- Assemble from memory in milliseconds; the IDE will rebuild on every run.

Syntax
- Motorola syntax only, one strict dialect close to vasm's Motorola syntax module.
- Accept a few ASM-One habits: labels without colons, blk.b/w/l, ; and * comments.
- Anything outside the dialect is a clear error. MIT/GNU syntax is out of scope.
- Not a goal: accepting every old demo source.

CPU support
- Data-driven table: instruction x size x addressing mode x CPU
  (68000, 010, 020, 030, 040, 060), FPU and MMU as separate axes.
  Source: M68000 Family Programmer's Reference Manual.
- Unsupported on target = error; 060-trapped instructions = warning.
- Leave room in the table for cycle counts later (exact for 68000).

Outputs
- Amiga hunk executable, raw binary (bootblocks, trackloaded code),
  object files. Linking later.
- Optional listing file.

Diagnostics and debugging
- Structured diagnostics with source spans (file, line, column),
  usable by an editor, not just printed text.
- Every emitted byte maps to its source line (address <-> line table)
  from day one. Export symbols and the line table via the library API.
- DWARF output later; the line table design must not block it.

Testing
- vasm is the byte-for-byte reference (unmodified, fetched/built by
  the test script, not committed: licence forbids modified redistribution).
- Phase 1: compare against vasm -no-opt, where every encoding is unambiguous.
- Phase 2: add optimisations/shortening one rule at a time, each matched
  to the corresponding vasm option.
- Generated tests covering every opcode, size and addressing mode per CPU;
  check CPU availability against vasm built for each -m680x0 target.
- A small corpus of clean real-world sources (Aminet) assembled whole.

Licence: GPL-compatible (it will ship with the GPL vAmiga core).
