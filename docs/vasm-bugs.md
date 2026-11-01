# vasm bugs found while testing rasm-one

vasm is rasm-one's byte-for-byte reference, but rasm-one does not copy its bugs: it
does the correct thing, and a test records the difference. This list is for
reporting them to the vasm authors. All were seen with vasm 2.0d, motorola syntax
module 3.19d, cpu backend 2.8.

This is not the place for vasm's permissive behaviour, where rasm-one is simply
stricter by choice. Those are the `STRICTER_THAN_VASM` lists in the tests.

## 1. Relocation on the wrong byte for an external in an indexed mode

**Source:** `xref x` / `move.w x(a0,d3.w),d0`, with `-Fhunk`.

**Bug:** the `HUNK_EXT` reference (type `0x8B`, 8-bit) points at offset 2, the extension
word's first byte, which holds the index register. The displacement is at offset 3.
The PC-relative form `x(pc,d0.w)` correctly gives offset 3.

**Effect:** linking with `vlink` and `x=$12` gives `3030 4200`, which is a different,
wrong instruction. The correct result is `3030 3012`.

**rasm-one:** writes offset 3.
`test_write_hunk_object_indexed_external_relocates_displacement_byte`.

## 2. Hangs on an include cycle

**Source:** a file that includes itself, directly or through another file.

**Bug:** vasm doesn't return. A test binary comparing two such cases took about 5 minutes
before they were removed, and a 3-second limit always stopped it.

**rasm-one:** reports `include-cycle`. Listed in `tests/vasm_include.rs`.

## 3. Negative counts in `ds` and `dcb`

**Source:** `ds.b -1` or `dcb.b -1,1`.

**Bug:** `ds.b -1` stops with `fatal error: out of heap memory`. `dcb.b -1,1` didn't
return within 60 seconds.

**rasm-one:** reports `bad-count`. `test_ds_negative_count_is_error`.

## 4. A suppressed PC still subtracts the PC

**Source:** `move.w (4,zpc),d0`, and `(4,zpc,d1)` and `([4,zpc])`, on 68020 and later.

**Bug:** vasm encodes the base displacement as 2, as if it were PC-relative, giving
`303b 01e0 0002`. With the PC suppressed, nothing is added to the displacement, so the
instruction reads address 2 rather than the 4 that was written.

**rasm-one:** writes the value as given, `303b 01e0 0004`.
`test_assemble_zpc_displacement_is_not_pc_relative`.

## 5. Valid long displacements rejected for some instructions

**Source:** on 68020 and later, `divs (100000,a6),d4`, the same with `divu`, `muls` and
`mulu`, and `jmp (100000,pc)` and `jsr (100000,pc)`.

**Bug:** vasm reports `displacement out of range` for these automatically sized full-format
modes. It accepts the same modes with other instructions, such as
`move.w (100000,a6),d0`, `lea (100000,pc),a0` and `pea (100000,pc)`.

**rasm-one:** encodes them, for example `divs (100000,a6),d4` as `89f6 0170 0001 86a0` and
`jmp (100000,pc)` as `4efb 0170 0001 869e`. `vasm_wrongly_rejects` in
`tests/sweep_cpus.rs`, and `test_encode_grown_modes_use_the_full_format_mode_bits`.

## 6. Relocation misplaced for an outer displacement after a word base displacement

**Source:** `move.w ([4,a0],f),d0` with `f` a label (`-Fhunkexe` or `-Fhunk`), or an external
`x` in its place, on 68020 and later.

**Bug:** the base displacement `4` takes one word, so the 32-bit outer displacement is at
offset 6, but vasm puts the relocation at offset 4, as if the base displacement were long.

**Effect:** linking the external form with `vlink` and `x=$1234` gives
`3030 0163 0004 1234 0000`, corrupting the base displacement and leaving the outer one
wrong. The correct result is `3030 0163 0004 0000 1234`.

**rasm-one:** puts the relocation at offset 6.
`test_write_hunk_exe_outer_displacement_reloc_follows_word_base`.
