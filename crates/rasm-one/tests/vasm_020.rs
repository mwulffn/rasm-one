//! Compares 68020 and later behaviour that depends on values with vasm -no-opt:
//! which form a displacement takes, in each output format.

use std::error::Error;

use rasm_one::{
    assemble, write_bin, write_hunk_exe, write_hunk_object, Cpu, MemoryProvider, Options,
};
use rasm_oracle::{Format, Vasm};

fn assembled(source: &str, cpu: Cpu, fixed_addresses: bool) -> rasm_one::Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::new("t.s");
    options.cpu = cpu;
    options.fixed_addresses = fixed_addresses;
    assemble(&options, &files)
}

const BINARY: &[&str] = &[
    // Displacements growing with their value.
    " move.w f(a0),d0\nf:",
    " move.w (f,a0),d0\nf:",
    " move.w f(a0),d0\n ds.b 40000\nf:",
    " move.w f-g(a0),d0\ng:\n ds.b 40000\nf:",
    " move.w g-f(a0),d0\nf:\n ds.b 40000\ng:",
    " move.w n(a0),d0\nn=100000",
    " move.w n(a0),d0\nn=4",
    " move.w n(a0,d1),d0\nn=4",
    " move.w n(a0,d1),d0\nn=1000",
    " move.w n(a0,d1),d0\nn=100000",
    " move.w f(a0,d1),d0\nf:",
    " move.w f(a0,d1),d0\n ds.b 1000\nf:",
    " move.w f(pc),d0\nf:",
    " move.w f(pc),d0\n ds.b 40000\nf:",
    " move.w f(pc,d1),d0\nf:",
    " move.w f(pc,d1),d0\n ds.b 1000\nf:",
    " move.w f(pc,d1),d0\n ds.b 100000\nf:",
    " move.w (f,pc,d1*2),d0\nf:",
    " lea f(pc),a0\n ds.b 40000\nf: nop",
    // A later instruction must see the earlier one's new length.
    " move.w f(a0),d0\n ds.b 40000\nf: dc.l f",
    "s: move.w f(pc,d1),d0\n bra.s s\n ds.b 1000\nf:",
    // The ends of each size.
    " move.w $7fff(a0),d0",
    " move.w $8000(a0),d0",
    " move.w -32768(a0),d0",
    " move.w -32769(a0),d0",
    " move.w $ffff8000(a0),d0",
    " move.w $ffff7fff(a0),d0",
    " move.w 127(a0,d1),d0",
    " move.w 128(a0,d1),d0",
    " move.w -128(a0,d1),d0",
    " move.w -129(a0,d1),d0",
    " move.w $ffffff80(a0,d1),d0",
    " move.w 32767(a0,d1),d0",
    " move.w 32768(a0,d1),d0",
    // Written sizes.
    " move.w (0,a0),d0",
    " move.w (0.w,a0),d0",
    " move.w (0.l,a0),d0",
    " move.w (0,a0,d1),d0",
    " move.w (0.w,a0,d1),d0",
    " move.w (4.w,pc),d0",
    " move.w (4.l,pc),d0",
    " move.w (4.w,pc,d1),d0",
    // Memory indirect.
    " move.w ([0,a0],d1,0),d0",
    " move.w ([0.w,a0],d1,0.w),d0",
    " move.w ([f,a0],d1,g),d0\nf:\ng:",
    " move.w ([100000,a0],d1,$ffff8000),d0",
    " move.w ([$8000,a0]),d0",
    " move.w ([-1,a0]),d0",
    " move.w ([f,pc]),d0\nf:",
    " move.w ([f,pc],d1,4),d0\n ds.b 40000\nf:",
    " move.w ([a0],a1),d0",
    " move.w ([4],d1),d0",
    " move.w ([4],8),d0",
    " move.w ([4,d1]),d0",
    " move.w ([d1]),d0",
    " move.w ([4,za0],zd1,8),d0",
    " jmp ([4,a0])",
    " lea (100000,a0),a1",
    // Other bases and indexes.
    " move.w (a1,a0),d0",
    " move.w (pc,a0),d0",
    " move.w (d1),d0",
    " move.w (d1.w*2),d0",
    " move.w (4,d1.l),d0",
    " move.w (zd1),d0",
    " move.w (za0),d0",
    " move.w (zpc),d0",
    " move.w (za0,d1),d0",
    " move.w (4,a0,zd1),d0",
    " move.w (4,sp,d1),d0",
    " move.w (zpc,d1),d0",
    "za0: nop\nzpc: nop\nzd0: nop",
    // Long branches.
    " bra.l f\nf:",
    " beq.l f\n ds.b 100000\nf:",
    "f: bsr.l f",
    // Long forms of older instructions.
    " link.l a6,#100000",
    " chk.l #1,d1",
    " tst.l a0",
    " tst.w #1",
    " tst.w 4(pc)",
    " cmpi.w #1,4(pc)",
    " cmpi.w #1,4(pc,d0)",
    " move ccr,(a0)",
    " extb d0",
    " muls.l #1,d1",
    " mulu d0,d1:d2",
    " divs d0,d1:d2",
    " divsl d0,d1:d2",
    " divul.l (a0),d1:d2",
    // Bit fields.
    " bfextu d0{1:2},d1",
    " bfextu (a0){d1:d2},d3",
    " bfexts (a0){1:d2},d3",
    " bfffo 4(a0){d1:2},d3",
    " bfins d3,(a0){1:2}",
    " bfchg (a0){1:32}",
    " bfclr d0{0:1}",
    " bfset (a0){31:32}",
    " bftst 4(pc){1:2}",
    " bftst f(pc){1:2}\n ds.b 40000\nf:",
    "n=3\n bfextu f(a0){n:n},d1\nf:",
    // Immediates in other places.
    " pack d0,d1,#1",
    " pack -(a0),-(a1),#$ffff",
    " unpk d0,d1,#$ffff",
    " unpk -(a0),-(a1),#1",
    " cas.b d0,d1,4(a0)",
    " cas2.w d0:d1,d2:d3,(a0):(a1)",
    " cas2.l d0:d1,d2:d3,(d0):(d1)",
    " chk2.l 4(a0),a1",
    " cmp2.b f(pc),d0\nf:",
    " callm #255,(a0)",
    " rtd #$ffff",
    " rtd #-4",
    " rtd #-32768",
    " bkpt #7",
    " trapeq",
    " trapne.w #$ffff",
    " trapcs.l #$12345678",
    " moves.b a1,4(a0)",
    " moves.l (a0),d0",
    // CPU selection in the source.
    " mc68020\n bfextu d0{1:2},d1",
    " opt p=68020\n bfextu d0{1:2},d1",
    " opt c-,p=68030\n pflusha",
    " mc68010\n movec vbr,d0",
    " machine 68020\n bfextu d0{1:2},d1",
    " machine mc68020\n bfextu d0{1:2},d1",
    " machine 68000\n nop",
    " mc68000\n nop",
    " mc68020\n nop\n mc68000\n nop",
];

const REJECTED_ON_68020: &[&str] = &[
    " bra.s *+1",
    " beq.s *+1",
    " rtd #65536",
    " rtd #-32769",
    " unpk d0,d1,#-1",
    " pack d0,d1,#$10000",
    " move.w (1000.b,a0,d1),d0",
    " move.w (100000.w,a0),d0",
    " move.w (d0.b),d0",
    " move.w (1,2,3),d0",
    " move.w ([],d1),d0",
    " move.w ([a0],[d1]),d0",
    " move.w ([a0,d1],a1),d0",
    " move.w ([a0,d1],d2),d0",
    " move.w ([a0],d1,d2),d0",
    " move.w (a0,d1.w*3),d0",
    " bra.w f\n ds.b 40000\nf:",
    " bra f\n ds.b 40000\nf:",
    " bsr f\n ds.b 100000\nf:",
    " link a0,#100000",
    " bfextu d0{32:1},d1",
    " bfextu d0{1:33},d1",
    " bkpt #8",
    " callm #256,(a0)",
    " movec $801,d0",
    " movec tc,d0",
    " mc68020\n nop\n mc68000\n bfextu d0{1:2},d1",
    " machine 99",
];

/// Lenient in vasm, and errors here. A bit field width of 0 encodes as 32, so
/// accepting it could hide a mistake.
const STRICTER_THAN_VASM: &[&str] = &[
    " bfextu d0{1:0},d1",
    " trapeq d1",
    " trapeq.b",
    " trapeq.w",
    " move.w (4.b,a0),d0",
    " move.w (4.b,a0,d1),d0",
    " move.w (d1,a0),d0",
    " machine",
    " cpu32",
];

/// Displacements that need a relocation take the long form, unlike in binaries.
const HUNK: &[&str] = &[
    " move.w f(a0),d0\nf:",
    " move.w f(a0,d1),d0\nf:",
    " move.w (f,a0,d1),d0\nf:",
    " move.w ([f,a0]),d0\nf:",
    " move.w (f.l,a0),d0\nf:",
    " move.w ([f,a0],d1,g),d0\nf:\ng:",
    " move.w f(pc,d1),d0\nf:",
    " move.w f(pc),d0\n ds.b 40000\nf:",
    " bra.l f\nf:",
    " nop\n lea (4.l,pc),a0",
    " nop\n lea (100000,pc),a0",
    " nop\n lea 100000(pc,d1),a0",
    " nop\n lea ([4,pc]),a0",
    " section a,code\n bra.l f\n section b,code\nf: nop",
    " section a,code\n nop\n bsr.l f\n bsr.l g\n bsr.l f\n section b,code\nf: nop\n section c,code\ng: nop",
];

const HUNK_REJECTED: &[&str] = &[
    " move.w (f.w,a0),d0\nf:",
    " section a,code\n lea f(pc),a0\n section b,code\nf: nop",
    " section a,code\n move.w f(pc,d0),d1\n section b,code\nf: nop",
];

const OBJECT: &[&str] = &[
    " move.w ([f,a0]),d0\nf:",
    " move.w f(a0),d0\nf:",
    " xref x\n move.w x(a0),d0",
    " xref x\n move.w x(pc),d0",
    " xref x\n move.w x(pc,d1),d0",
    " xref x\n bra.l x",
    " xref x\n bsr x",
    " xref x\n move.w ([x,a0],d1,x),d0",
    " xref x\n move.w (x.l,pc),d0",
    " xref x\n move.w ([x,pc]),d0",
    " xref x\n move.w ([x,pc],d0,2),d1",
    " section a,code\n bra.l f\n section b,code\nf: nop",
    " section a,code\n lea f(pc),a0\n section b,code\nf: nop",
    " section a,code\n nop\n bsr.l f\n bsr.l g\n bsr.l f\n section b,code\nf: nop\n section c,code\ng: nop",
];

fn compare(
    vasm: &Vasm,
    sources: &[&str],
    format: Format,
    write: &dyn Fn(&rasm_one::Assembly) -> Result<Vec<u8>, rasm_one::OutputError>,
    fixed_addresses: bool,
) -> Result<Vec<String>, Box<dyn Error>> {
    let mut failures = Vec::new();
    for source in sources {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68020", format)?;
        if !theirs.success {
            failures.push(format!("vasm rejected {source:?}: {}", theirs.stderr));
            continue;
        }
        let assembly = assembled(&source, Cpu::M68020, fixed_addresses);
        match write(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours {}\n  vasm {}",
                hex(&bytes),
                hex(&theirs.bytes)
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    Ok(failures)
}

/// The last 48 bytes: relocations follow the code, and a difference in the code
/// shows up in the length too.
fn hex(bytes: &[u8]) -> String {
    let shown = &bytes[bytes.len().saturating_sub(48)..];
    shown
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .concat()
}

#[test]
fn test_write_bin_on_68020_matches_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let failures = compare(&vasm, BINARY, Format::Bin, &|a| write_bin(a), true)?;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_exe_on_68020_matches_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let failures = compare(&vasm, HUNK, Format::HunkExe, &|a| write_hunk_exe(a), false)?;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_object_on_68020_matches_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let failures = compare(
        &vasm,
        OBJECT,
        Format::HunkObject,
        &|a| write_hunk_object(a),
        false,
    )?;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_assemble_on_68020_rejects_what_vasm_rejects() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED_ON_68020 {
        let source = format!("{source}\n");
        if vasm.assemble(&source, "68020", Format::Bin)?.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if !assembled(&source, Cpu::M68020, true).has_errors() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    for source in HUNK_REJECTED {
        let source = format!("{source}\n");
        if vasm.assemble(&source, "68020", Format::HunkExe)?.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if write_hunk_exe(&assembled(&source, Cpu::M68020, false)).is_ok() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_assemble_on_68020_stricter_than_vasm_sources_are_errors() {
    for source in STRICTER_THAN_VASM {
        let source = format!("{source}\n");
        assert!(
            assembled(&source, Cpu::M68020, true).has_errors(),
            "{source:?}"
        );
    }
}

/// One reviewed difference in bytes, listed in docs/vasm-bugs.md. The outer
/// displacement is at offset 6, not at the 4 that vasm's relocation points to.
#[test]
fn test_write_hunk_exe_outer_displacement_reloc_follows_word_base() -> Result<(), Box<dyn Error>> {
    let assembly = assembled(" move.w ([4,a0],f),d0\nf:\n", Cpu::M68020, false);
    let relocs = &assembly.sections[0].relocs;
    assert_eq!((relocs[0].offset, relocs[0].size), (6, 4));
    write_hunk_exe(&assembly)?;
    Ok(())
}

#[test]
fn test_machine_directive_only_changes_later_lines() {
    let assembly = assembled(
        " bfextu d0{1:2},d1\n mc68020\n bfextu d0{1:2},d1\n",
        Cpu::M68000,
        true,
    );
    assert_eq!(assembly.diagnostics.len(), 1);
    assert_eq!(assembly.diagnostics[0].span.map(|s| s.line), Some(1));
}
