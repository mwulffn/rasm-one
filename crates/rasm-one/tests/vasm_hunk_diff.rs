//! Compares hunk executables with vasm -no-opt byte for byte.

use std::error::Error;

use rasm_one::{
    assemble, write_hunk_exe, write_hunk_exe_with, HunkExeOptions, MemoryProvider, Options,
    OutputError,
};
use rasm_oracle::{Format, Vasm};

fn ours(source: &str) -> Result<Vec<u8>, OutputError> {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    write_hunk_exe(&assemble(&Options::new("t.s"), &files))
}

const ACCEPTED: &[&str] = &[
    // Padding.
    "",
    " nop",
    " dc.b 1",
    " dc.b 1,2",
    " dc.b 1,2,3",
    " dc.b 1,2,3,4,5,6",
    " dc.b 1,2,3,4,5,6,7",
    " section d,data\n dc.b 1,2",
    // Symbols.
    "a: nop\nb: rts",
    "abcd: nop",
    "abcde: nop",
    "abcdefgh: nop",
    "start: lea data(pc),a0\n bsr sub\n rts\nsub: rts\ndata: dc.w 1",
    "l: dc.b 1\n dc.b 2\nm: dc.w m-l\n dc.l m+1,l",
    // Section kinds and memory flags.
    " section c,code\ns: dc.l s\n dc.b 1",
    " section d,data\ns: dc.b 1",
    " section b,bss\ns: ds.b 5",
    " section c,code_c\n nop\n section d,data_f\n dc.w 1\n section b,bss_c\n ds.l 1",
    " section c,code,chip\n nop\n section d,data,fast\n dc.w 1",
    " section a\n nop",
    " section a",
    // Which sections become hunks.
    " section e1,code\n section e2,data\n dc.w 1",
    " section a,code\nl:\n section b,data\n dc.l l",
    " section e,bss\nl:",
    " section e,code\nl:\n section f,code\n nop",
    " section a,code\n section e,data\n section b,bss\n dc.l 0\n section c,code\nx: dc.l x",
    // Section identity.
    " nop\n section a,code\n rts",
    " nop\n section CODE,code\n rts",
    " nop\n section code,code\n rts",
    " section c,code\ns: nop\n section c,data\n dc.w 1",
    " section c,code\n nop\n section c,code_c\n rts",
    " section c,code\n nop\n section d,data\n dc.w 1\n section c,code\n rts",
    // Initialised data in BSS is dropped.
    " section b,bss\ns: dc.l s",
    " section b,bss\n ds.b 3\n dc.b 1",
    // Directives.
    " rorg 4\nl: dc.l l",
    "a equ 4\n nop",
    "a set 4\n nop",
    "a=4\nl: nop\nb equ l+2\n dc.l b",
    " section x,code\nl: nop\n section y,data\n dc.l m\nm equ l",
    "r reg d0\n nop",
    "r equr d0\n nop",
    "x rs.w 1\n nop",
    " dc.b 1\n even\n nop",
    " nop\n cnop 0,4\n nop",
    " dc.b 1\n cnop 0,4\n dc.b 2",
    " section x,data\n dc.b 1\n cnop 0,4\n dc.b 2",
    " section x,bss\n ds.b 1\n cnop 0,4\n ds.b 1",
    " nop\n align 3\n nop",
    " code\n nop\n data\n dc.w 1\n bss\n ds.w 1",
    " code_c\n nop\n data_c\n dc.w 1\n bss_c\n ds.w 1",
    " code\n nop\n section CODE,code\n rts\n data\n dc.w 1\n section DATA,data\n dc.w 2",
    " code\n nop\n code_c\n rts",
    " bss\n ds.w 1\n section BSS,bss\n ds.w 1",
    "x equ 1\nl: nop\ny equ 2",
    "n equ 1\nl: nop\n dc.l l+n",
    // A constant PC displacement is used as it is, wherever the instruction is.
    " nop\n nop\n jmp 2(pc,d0.w)\n move.w 4(pc),d0\n lea 6(pc),a0",
    " nop\n lea 4(pc),a0",
    // Local labels on an instruction's line belong to the label before them.
    "foo: nop\n.x: nop\nbar: bra .x\n.x: rts",
    "foo: nop\n1$: nop\nbar: bra 1$\n1$: rts",
    "foo: nop\n.x: nop\nbar: lea .x(pc),a0\n.x: rts",
    // opt settings that change the file.
    "start: nop\nb: nop\n xdef start\n opt d-",
    " opt d-\nstart: nop\nb: nop\n opt d+",
    " opt x+\nstart: nop\nb: nop\n xdef start",
    " opt x+\nstart: nop\nb: nop\n xdef start\n opt x-",
    " opt x+,d-\nstart: nop\nb: nop\n xdef start",
    "start: nop\nb: nop\n xdef start\n opt l+",
    " opt l+\n xref y\n dc.l y",
    " opt d\nstart: nop",
    " opt x\nstart: nop",
    " opt l\nstart: nop",
    " opt p-\nstart: nop",
    " opt nodebug\nstart: nop",
    " opt debug\nstart: nop",
    " opt xdebug\nstart: nop\nb: nop\n xdef start",
    " opt noxdebug\nstart: nop",
    " opt l0\nstart: nop",
    " opt l1\nstart: nop",
    " opt l2\nstart: nop",
    // Relocations.
    " move.l #s,s\ns: dc.l s",
    " move.l s,s\n dc.l s\ns:",
    " dc.b 1\n dc.l s\ns:",
    " section c,code\ns: dc.l s,t,s,t+2\n section d,data\nt: dc.l s,t,u\n section b,bss\nu: ds.w 1",
    " dc.l s\n ds.b 65534\ns: dc.l s",
    " ds.b 65532\ns: dc.l s\n dc.l s",
    " dc.l s\n ds.b 65532\n dc.l s\ns:\n section d,data\n dc.l s",
    " move.l #s,d0\n lea s,a0\n jmp_here: bra jmp_here\ns: dc.l s",
];

const REJECTED: &[&str] = &[
    " move.w f.w,d0\nf:",
    " move.w f(a0),d0\nf:",
    " move.w f(a0,d0.w),d0\nf:",
    " move.b #f,d0\nf:",
    " move.w #f,d0\nf:",
    " dc.w f\nf:",
    " dc.b f\nf:",
    " section c,code\n lea t(pc),a0\n section d,data\nt: dc.w 1",
    " section c,code\n bsr t\n section e,code\nt: rts",
    " dc.l 5-s\ns:",
    " dc.l -s\ns:",
];

#[test]
fn test_write_hunk_exe_accepted_sources_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in ACCEPTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::HunkExe)?;
        assert!(
            theirs.success,
            "vasm rejected {source:?}: {}",
            theirs.stderr
        );
        match ours(&source) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {bytes:02x?}\n  vasm  {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_exe_kickstart1_matches_vasm_kick1hunks() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in ACCEPTED {
        let source = format!("{source}\n");
        let theirs =
            vasm.assemble_with_args(&source, "68000", Format::HunkExe, &["-kick1hunks"])?;
        assert!(
            theirs.success,
            "vasm rejected {source:?}: {}",
            theirs.stderr
        );
        let mut files = MemoryProvider::new();
        files.insert("t.s", source.as_str());
        let options = HunkExeOptions { kickstart1: true };
        match write_hunk_exe_with(&assemble(&Options::new("t.s"), &files), options) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {bytes:02x?}\n  vasm  {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_exe_rejected_sources_match_vasm_rejection() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::HunkExe)?;
        assert!(!theirs.success, "vasm accepted {source:?}");
        if ours(&source).is_ok() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "we accepted {failures:?}");
    Ok(())
}

/// See docs/vasm-bugs.md: vasm writes a displacement of 0 for these.
#[test]
fn test_write_hunk_exe_branch_to_constant_is_error() {
    for source in [" bra 4\n", " dbra d0,4\n", " bsr.s 4\n"] {
        assert!(ours(source).is_err(), "{source}");
    }
}

#[test]
fn test_write_hunk_exe_narrow_reloc_error_has_source_line() {
    match ours(" nop\n move.w f.w,d0\nf:\n") {
        Err(OutputError::UnsupportedReloc { size, span, .. }) => {
            assert_eq!((size, span.line), (2, 2));
        }
        other => panic!("expected an unsupported relocation, got {other:?}"),
    }
}
