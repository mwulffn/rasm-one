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

The rejection also applies to other values that don't fit a word, such as
`divu 65535(a6),d7`, and `.l` on the displacement avoids it.

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

## 7. Floating-point values stored with the wrong meaning

Not everything here is a bug. vasm deliberately stores hex, octal, binary and symbol values
as raw bits in floating-point data (`dc.s $3f800000`), while decimal expressions are numbers
(`dc.s 1+1` is 2.0). rasm-one follows that. These go beyond it, with no message:

| Source | vasm writes | Problem |
|---|---|---|
| `fmove.l #1.5,fp0` | `f23c 4000 3fc0 0000` | The single-precision bits of 1.5, loaded as the integer 1069547520 |
| `dc.l 1.5` | `3fc00000` | The same, in data |
| `dc.s 1e50` | `7f800000` | Infinity, with no out-of-range message |
| `dcb.s 1,1.5` | `00000001` | The `.5` is dropped |

**rasm-one:** all are errors. `tests/vasm_fpu.rs`.

## 8. Extended precision depends on the host (limitation, not a logic bug)

**Source:** `dc.x 0.1`.

**Behaviour:** on an ARM Mac, where `long double` is 64 bits, vasm gives
`3ffb0000ccccccccccccd000`, a double-precision value widened to 80 bits. The correctly rounded
value is `3ffb0000cccccccccccccccd`. Results for values that a double can't represent
exactly therefore differ between hosts.

**rasm-one:** converts exactly on every host, so its comparison tests only use values that a
double can represent. `test_float_bytes_extended_needs_full_precision`.

## 9. Internal error on a raw packed value

**Source:** `dc.p $1`.

**Bug:** `fatal error 5: internal error 0 in line 5607`, where `fmove.p #$1,fp0` works.

**rasm-one:** writes the raw bits, as with the other floating-point sizes.
`test_dc_p_raw_value_is_written`.

## 10. A branch to a constant in a relocatable section is silently wrong

**Source:** `bra 4` with `-Fhunkexe` or `-Fhunk`, and `dbra d0,4` and `bra.l 4`.

**Bug:** vasm writes a displacement of 0, `6000 0000`, with no message and no relocation, so the
branch goes to the next instruction rather than anywhere near address 4. `bsr.s 4` does report
an error.

**rasm-one:** reports `absolute-branch`, since the distance isn't known.
`test_write_hunk_exe_branch_to_constant_is_error`.

## 11. Values over 32 bits saturate silently

**Source:** `jmp $100000000`, also `jmp -$80000001`, `bra.l $123456789` and
`move.l (100000000000,a0),d0`.

**Bug:** vasm writes `7fffffff`, with no message. `dc.l` gives a warning for the same thing.

**rasm-one:** reports `out-of-range`. `test_values_over_32_bits_are_errors`.

## 12. `opt c-` doesn't disable case sensitivity

**Source:** `opt c-` (or `opt nocase`) / `Foo: nop` / `dc.l foo`.

**Bug:** vasm's manual documents the option as turning case sensitivity off, but the source
still fails with `undefined symbol <foo>`.

**rasm-one:** symbols stay case-sensitive, as they are in vasm, but it warns
`option-ignored` so the source's author knows.

## 13. An unsized branch one byte ahead is rejected on 68020 and later

**Source:** `bra *+1` with `-m68020 -opt-o1+`.

**Bug:** vasm picks the short form, whose displacement of -1 means a long displacement on these
CPUs, and reports `branch destination out of range`. `bra.w *+1` assembles to `6000 ffff`.

**rasm-one:** writes the word form. `test_assemble_branch_to_odd_next_byte_uses_word_on_68020`.
