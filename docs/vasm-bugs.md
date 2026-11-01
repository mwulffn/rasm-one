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
