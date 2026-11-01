//! Compares hunk object files with vasm -no-opt byte for byte.

use std::error::Error;

use rasm_one::{
    assemble, write_bin, write_hunk_exe, write_hunk_object, Assembly, MemoryProvider, Options,
    OutputError,
};
use rasm_oracle::{Format, Vasm};

fn assembled(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    assemble(&Options::new("t.s"), &files)
}

const ACCEPTED: &[&str] = &[
    "",
    " nop",
    "a: nop\nb: rts",
    " section c,code\ns: dc.l s\n dc.b 1",
    " section d,data\n dc.b 1",
    " section d,data\n dc.b 1,2",
    " section b,bss\ns: ds.b 5",
    " section c,code_c\n nop\n section d,data_f\n dc.w 1\n section b,bss_c\n ds.l 1",
    " section abcd,code\n nop",
    " section abcde,code\n nop",
    " section a,code\n section b,data\n dc.w 1",
    " section c,code\ns: dc.l s,t\n section d,data\nt: dc.l s,t+2\n section e,bss\nu: ds.w 1\n section c,code\n dc.l u",
    " dc.l s\n ds.b 70000\ns: dc.l s",
    // References to other objects.
    " xref x\n dc.l x",
    " xref x",
    " xref x,y\n dc.l x,y,x+2",
    " xref y,x\n dc.l x,y",
    " xref x,y\n dc.l y,x,y",
    " xref x\n dc.w x\n dc.l x\n dc.w x",
    " xref x,y\n dc.w x\n dc.l y\n dc.l x\n dc.w y",
    " xref x\n move.l x,d0",
    " xref x\n move.w x.w,d0",
    " xref x\n move.w x(a0),d0",
    " xref x\n move.b #x,d0",
    " xref x\n move.w #x,d0",
    " xref x\n dc.w x",
    " xref x\n dc.b x",
    " xref x\n bsr x",
    " xref x\n bsr x+2",
    " xref x\n bra.s x",
    " xref x\n bsr.s x+2",
    " xref x\n lea x(pc),a0",
    " xref x\n move.w x+2(pc),d0",
    " xref x\n move.w x(pc,d0.w),d0",
    " xref x\n dbra d0,x",
    " xref x\n dc.w x+1,x+2",
    " xref x\n dc.l x-1",
    " xref x\n dc.l 1+x",
    " xref x\n section b,bss\n dc.l x",
    " XREF x\n dc.l x",
    // Exports.
    " xdef a\na: nop",
    " xdef a,b\na: nop\nb: nop",
    " xdef b,a\na: nop\nb: nop",
    " xdef a\nb: nop\na: nop\n xdef b",
    "b: nop\na: nop\n xdef a,b",
    " xdef a\na=4",
    " xdef a\na=4\n nop",
    " xdef a\n section x,data\nb: dc.w 1\na: dc.w 2",
    " public a\na: nop",
    "a:: nop",
    " a:: nop",
    "a:: nop\nb:: nop\n xref x\n dc.l x",
    " xdef abcdefgh\nabcdefgh: nop",
    " xref abcd\n dc.l abcd",
    "r equ 1\n xdef r",
    "l: nop\nb equ l+2\n xdef b",
    " xdef a\n section s1,code\n nop\n section s2,code\n nop\na=4",
    " section s1,code\n nop\n section s2,code\n xdef a\na=4\n nop",
    " section s1,code\n nop\n section s2,code\n xdef a,b\nb: nop\na=4",
    " section b,bss\ns: ds.b 2\n xdef s",
    " section b,bss\ns: dc.l s",
    "a: nop\nb:\n xdef b\n xref x\n dc.l x,a\n section s2,code\n xref y\n dc.l x,y",
    // PC-relative references to other sections.
    " section c,code\n lea t(pc),a0\n section d,code\nt: dc.w 1",
    " section c,code\n bsr t\n section d,code\nt: rts",
    " section c,code\n nop\n lea t(pc),a0\n section d,code\n dc.w 1\nt: dc.w 1",
    " section c,code\n nop\n move.w t(pc,d0.w),d1\n section d,code\n dc.w 1\nt: dc.w 1",
    " section c,code\n nop\n bsr.s t\n section d,code\n dc.w 1\nt: dc.w 1",
    " section c,code\n nop\n bsr t\n bsr u\n bsr t\n section d,code\nt: dc.w 1\n section e,code\nu: dc.w 1",
    // Macros and local labels.
    "a: nop\n.l: nop\n xref x\n dc.l .l,x",
    "m macro\n xdef \\1\n\\1: nop\n endm\n m foo\n m bar",
];

const REJECTED: &[&str] = &[
    " xref x,y\n dc.l x-y",
    " xref x\n dc.l x*2",
    " xref x\n ds.b x",
    " xdef",
    " xref",
    " section c,code\n dc.w t\n section d,code\nt: rts",
    " section c,code\n dc.b t\n section d,code\nt: rts",
];

/// vasm accepts these, at most with a warning. An undefined symbol becoming an
/// external would turn typos into link errors, as vasm's own -x option agrees.
const STRICTER_THAN_VASM: &[&str] = &[
    " dc.l undefinedy",
    " xref x\n dc.l x-y\ny:",
    " xref x\nx: nop",
    " xref x\nx=1",
    "x: nop\n xref x",
    " xdef x\n xref x\nx=1",
    " xref x\n xdef x",
    " xdef a",
    " xdef x.y",
    " xdef .l\n.l: nop",
    ".l:: nop",
    " xref .l",
    " xdef.l a",
    " org 4\n nop",
];

#[test]
fn test_write_hunk_object_accepted_sources_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in ACCEPTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::HunkObject)?;
        if !theirs.success {
            failures.push(format!("vasm rejected {source:?}: {}", theirs.stderr));
            continue;
        }
        let assembly = assembled(&source);
        match write_hunk_object(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {}\n  vasm  {}",
                hex(&bytes),
                hex(&theirs.bytes)
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .chunks(4)
        .map(|chunk| {
            chunk
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .concat()
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[test]
fn test_write_hunk_object_rejected_sources_match_vasm_rejection() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::HunkObject)?;
        if theirs.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if write_hunk_object(&assembled(&source)).is_ok() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_object_stricter_than_vasm_sources_fail() {
    for source in STRICTER_THAN_VASM {
        let source = format!("{source}\n");
        assert!(
            write_hunk_object(&assembled(&source)).is_err(),
            "{source:?}"
        );
    }
}

/// The one reviewed difference in bytes. vasm places this relocation at offset
/// 2, the index register byte, so vlink corrupts the instruction: linking with
/// `x=$12` gives `3030 4200` where `3030 3012` is correct. The field is at 3.
#[test]
fn test_write_hunk_object_indexed_external_relocates_displacement_byte(
) -> Result<(), Box<dyn Error>> {
    let bytes = write_hunk_object(&assembled(" xref x\n move.w x(a0,d3.w),d0\n"))?;
    let reference_offset = &bytes[bytes.len() - 12..bytes.len() - 8];
    assert_eq!(reference_offset, [0, 0, 0, 3]);
    Ok(())
}

#[test]
fn test_write_other_formats_refuse_external_symbols() {
    let assembly = assembled(" xref x\n nop\n dc.l x\n");
    let is_external = |result: Result<Vec<u8>, OutputError>| {
        matches!(
            result,
            Err(OutputError::UnresolvedExternal { name, span })
                if name == "x" && span.line == 3
        )
    };
    assert!(is_external(write_hunk_exe(&assembly)));
    assert!(is_external(write_bin(&assembly)));
}

#[test]
fn test_write_hunk_object_unit_name_has_no_directory() -> Result<(), Box<dyn Error>> {
    let mut files = MemoryProvider::new();
    files.insert("src/n.s", " nop\n");
    let bytes = write_hunk_object(&assemble(&Options::new("src/n.s"), &files))?;
    assert_eq!(&bytes[8..12], b"n.s\0");
    Ok(())
}
