//! Compares FPU instructions and floating-point data with vasm -no-opt.
//!
//! Only values a double can represent exactly are compared: see docs/vasm-bugs.md
//! for why vasm's extended precision depends on the host.

use std::error::Error;

use rasm_one::{assemble, write_bin, Assembly, Cpu, FpuChoice, MemoryProvider, Options};
use rasm_oracle::{Format, Vasm};

fn ours(source: &str, cpu: Cpu, fpu: FpuChoice) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::new("t.s");
    options.cpu = cpu;
    options.fpu = fpu;
    options.fixed_addresses = true;
    assemble(&options, &files)
}

const ON_68040: &[&str] = &[
    // fmove and its sizes.
    " fmove fp0,fp1",
    " fmove.x fp0,fp1",
    " fmove.s #1.5,fp0",
    " fmove.d #1.5,fp0",
    " fmove.x #1.5,fp0",
    " fmove.p #1,fp0",
    " fmove.l #1,fp0",
    " fmove.w #1,fp0",
    " fmove.b #1,fp0",
    " fmove.s #1,fp0",
    " fmove.x #1,fp0",
    " fmove #1,fp0",
    " fmove d0,fp1",
    " fmove.l d0,fp1",
    " fmove.s d0,fp1",
    " fmove (a0),fp1",
    " fmove.l (a0),fp1",
    " fmove fp1,d0",
    " fmove.l fp1,d0",
    " fmove fp1,(a0)",
    " fmove.s fp1,(a0)",
    " fmove.x fp1,(a0)",
    " fmove.p fp1,(a0)",
    " fmove.p fp1,(a0){#3}",
    " fmove.p fp1,(a0){#-3}",
    " fmove.p fp1,(a0){d1}",
    " fmove fp1,(a0){#3}",
    " fmove.d fp1,4(a0,d1.w)",
    " fmove.s f(pc),fp0\nf: dc.s 1.5",
    " fmove.x (100000,a0),fp0",
    // Control registers.
    " fmove.l fpcr,d0",
    " fmove fpsr,d0",
    " fmove.l fpiar,a0",
    " fmove.l d0,fpcr",
    " fmove.l (a0),fpsr",
    " fmove a0,fpiar",
    " fmove.l #1,fpcr",
    " fmovem.l fpcr/fpsr,(a0)",
    " fmovem.l (a0),fpcr/fpsr/fpiar",
    " fmovem.l #1,fpcr",
    " fmovem fpcr,d0",
    " fmovem.l #2,fpcr/fpsr",
    " fmovem.l #2,fpsr/fpiar",
    " fmovem.l #$12345678,fpcr/fpsr/fpiar",
    // fmovem.
    " fmovem.x fp0-fp3,-(a0)",
    " fmovem.x (a0)+,fp0/fp2",
    " fmovem.x d1,-(a0)",
    " fmovem.x (a0)+,d1",
    " fmovem fp0,(a0)",
    " fmovem (a0),fp7",
    " fmovem.x #$ff,-(a0)",
    " fmovem.x fp3-fp1,4(a0)",
    " fmovem.x (a0),#$ff",
    // Other forms.
    " fmovecr #0,fp0",
    " fmovecr.x #$40,fp1",
    " fabs fp0",
    " fabs fp0,fp1",
    " fabs (a0),fp1",
    " fabs.s (a0),fp1",
    " fadd.l d0,fp1",
    " fadd.x fp0,fp1",
    " fsincos fp0,fp1:fp2",
    " fsincos.x (a0),fp1:fp2",
    " fsincos.l d3,fp1:fp2",
    " ftst fp0",
    " ftst.l d0",
    " ftst (a0)",
    " ftst #1",
    " fcmp fp0,fp1",
    " fnop",
    " fsave (a0)",
    " fsave -(a0)",
    " frestore (a0)+",
    " fsabs fp0",
    " fdadd fp0,fp1",
    " fssqrt fp0",
    " fdiv fp0,fp1",
    " fsgldiv fp0,fp1",
    // Branches and sets.
    " fbeq *+4",
    " fbeq.w *+4",
    " fbeq.l *+4",
    " fbf *+4",
    " fbt *+4",
    " fbngl *+4",
    " fbne f\n ds.b 1000\nf:",
    " fdbeq d0,*+4",
    " fdbne d3,f\nf:",
    " fseq d0",
    " fseq (a0)",
    " fseq.b 4(a0)",
    " ftrapeq",
    " ftrapeq.w #1",
    " ftrapne.l #1",
    // Floating-point data.
    " dc.s 1",
    " dc.s 1+1",
    " dc.s (1)",
    " dc.s -1",
    " dc.s +1",
    " dc.s 2*3",
    " dc.d 1+1",
    " dc.d -1",
    " dc.x 1+1",
    " dc.x -1",
    " dc.p 1+1",
    // Everything but decimal numbers supplies the raw bits.
    " dc.s $3f800000",
    " dc.s %1",
    " dc.s @1",
    " dc.d $1",
    " dc.d $4000000000000000",
    " dc.x $1",
    "a=1\n dc.s a",
    "a=1\n dc.d a",
    "a=1\n dc.x a",
    " fmove.s #$3f800000,fp0",
    " fmove.d #$ff00,fp0",
    " fmove.x #$1,fp0",
    " fmove.p #$1,fp0",
    " fmove.s #1+1,fp0",
    " fmove.s #-1,fp0",
    " fmove.x #1+1,fp0",
    "a=1\n fmove.s #a,fp0",
    " dcb.s 1,$3f800000",
    " dcb.s 1,-1",
    " dcb.s 1,1",
    " blk.s 1,1",
    " dc.s 1.5e2,-0.5",
    " dc.d 1.5,-2",
    " dc.d 0.1",
    " dc.x 1.5,-2",
    " dc.x 1",
    " dc.p 1,1.5,100",
    " dc.p -0.25",
    " dc.p 1e100",
    " dc.q 1,-1",
    " dc.q $123456789abcdef0",
    " ds.s 1",
    " ds.d 1",
    " ds.x 1",
    " ds.p 1",
    " ds.q 1",
    " rs.d 1\nx rs.b 0\n dc.b x",
    " rs.x 1\nx rs.b 0\n dc.b x",
    "m macro\n dc.\\0 1.5\n endm\n m.s\n m.d",
];

/// All 32 conditions, which the sweep only samples.
const CONDITIONS: &[&str] = &[
    "f", "eq", "ogt", "oge", "olt", "ole", "ogl", "or", "un", "ueq", "ugt", "uge", "ult", "ule",
    "ne", "t", "sf", "seq", "gt", "ge", "lt", "le", "gl", "gle", "ngle", "ngl", "nle", "nlt",
    "nge", "ngt", "sne", "st",
];

const REJECTED_ON_68040: &[&str] = &[
    " fmove.x d0,fp1",
    " fmove.d d0,fp1",
    " fmove fp1,4(pc)",
    " fmove #1.5,fp0",
    " fmove.w #1.5,fp0",
    " fmove.b #1.5,fp0",
    " fbne.s *+4",
    " fmove.l fpcr,a0",
    " fmove.l a0,fpsr",
    " fmovem.l fpcr/fpsr,d0",
    " dc.w 1.5",
    " dc.b 1.5",
];

/// vasm writes something wrong for these, or ignores part of them. See
/// docs/vasm-bugs.md.
const WRONG_IN_VASM: &[&str] = &[
    " fmove.l #1.5,fp0",
    " dc.l 1.5",
    " dc.s f\nf:",
    " dc.s 1/2",
    " fmove.s #1/2,fp0",
    " dc.s 1e50",
    " dcb.s 1,1.5",
    " blk.s 1,1.5",
    " dc.q f\nf:",
];

#[test]
fn test_fpu_forms_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    let conditions = CONDITIONS.iter().flat_map(|c| {
        [
            format!(" fb{c} *+6"),
            format!(" fb{c}.l *+8"),
            format!(" fdb{c} d1,*+8"),
            format!(" fs{c} (a1)"),
            format!(" ftrap{c}"),
            format!(" ftrap{c}.w #2"),
            format!(" ftrap{c}.l #2"),
        ]
    });
    let sources: Vec<String> = ON_68040
        .iter()
        .map(ToString::to_string)
        .chain(conditions)
        .collect();
    for source in &sources {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68040", Format::Bin)?;
        if !theirs.success {
            failures.push(format!("vasm rejected {source:?}: {}", theirs.stderr));
            continue;
        }
        let assembly = ours(&source, Cpu::M68040, FpuChoice::BuiltIn);
        match write_bin(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours {bytes:02x?}\n  vasm {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_fpu_rejected_forms_match_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED_ON_68040 {
        let source = format!("{source}\n");
        if vasm.assemble(&source, "68040", Format::Bin)?.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if !ours(&source, Cpu::M68040, FpuChoice::BuiltIn).has_errors() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_fpu_values_vasm_gets_wrong_are_errors() {
    for source in WRONG_IN_VASM {
        let source = format!("{source}\n");
        let assembly = ours(&source, Cpu::M68040, FpuChoice::BuiltIn);
        assert!(assembly.has_errors(), "{source:?}");
    }
}

#[test]
fn test_dc_p_raw_value_is_written() -> Result<(), Box<dyn Error>> {
    let bytes = write_bin(&ours(" dc.p $1\n", Cpu::M68040, FpuChoice::BuiltIn))?;
    assert_eq!(bytes, [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    Ok(())
}

#[test]
fn test_fpu_needs_built_in_or_coprocessor() {
    let accepts = |cpu, fpu| !ours(" fnop\n", cpu, fpu).has_errors();
    assert!(!accepts(Cpu::M68000, FpuChoice::BuiltIn));
    assert!(!accepts(Cpu::M68020, FpuChoice::BuiltIn));
    assert!(!accepts(Cpu::M68030, FpuChoice::BuiltIn));
    assert!(accepts(Cpu::M68040, FpuChoice::BuiltIn));
    assert!(accepts(Cpu::M68060, FpuChoice::BuiltIn));
    assert!(accepts(Cpu::M68020, FpuChoice::Coprocessor));
    assert!(accepts(Cpu::M68030, FpuChoice::Coprocessor));
    assert!(!accepts(Cpu::M68040, FpuChoice::None));
    // A coprocessor needs the 68020's interface, though vasm allows this.
    assert!(!accepts(Cpu::M68000, FpuChoice::Coprocessor));
    assert!(!accepts(Cpu::M68010, FpuChoice::Coprocessor));
}

#[test]
fn test_fpu_directives_enable_it_from_there_on() {
    let source = " fnop\n mc68881\n fnop\n fpu 0\n fnop\n fpu 1\n fnop\n machine 68882\n fnop\n";
    let assembly = ours(source, Cpu::M68020, FpuChoice::BuiltIn);
    let lines: Vec<_> = assembly
        .diagnostics
        .iter()
        .filter_map(|d| d.span.map(|s| s.line))
        .collect();
    assert_eq!(lines, [1, 5]);
}

#[test]
fn test_fpu_names_are_ordinary_symbols_elsewhere() -> Result<(), Box<dyn Error>> {
    let bytes = write_bin(&ours(
        "fp0: nop\nfpcr: nop\n dc.l fp0,fpcr\n",
        Cpu::M68000,
        FpuChoice::BuiltIn,
    ))?;
    assert_eq!(&bytes[4..], [0, 0, 0, 0, 0, 0, 0, 2]);
    Ok(())
}
