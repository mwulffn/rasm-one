//! The 68060 emulates some instructions through exceptions, listed in the MC68060
//! User's Manual, Appendix C. These warnings have no vasm reference, as vasm doesn't
//! warn about them.

use rasm_one::{assemble, Assembly, Cpu, MemoryProvider, Options, Severity};

fn ours(source: &str, cpu: Cpu) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", format!("{source}\n"));
    let mut options = Options::new("t.s");
    options.cpu = cpu;
    assemble(&options, &files)
}

fn warns(source: &str, cpu: Cpu) -> bool {
    let assembly = ours(source, cpu);
    assert!(
        !assembly.has_errors(),
        "{source}: {:?}",
        assembly.diagnostics
    );
    assembly
        .diagnostics
        .iter()
        .any(|d| d.code == "trapped-on-68060" && d.severity == Severity::Warning)
}

const TRAPPED: &[&str] = &[
    " divu.l d0,d1:d2",
    " divs.l (a0),d1:d2",
    " mulu.l d0,d1:d2",
    " muls.l #1,d1:d2",
    " muls d0,d1:d2",
    " movep.w d0,4(a0)",
    " movep.l 4(a0),d0",
    " cas2.w d0:d1,d2:d3,(a0):(a1)",
    " chk2.w (a0),d0",
    " cmp2.l (a0),a1",
    " facos fp0",
    " fasin fp0,fp1",
    " fatan (a0),fp1",
    " fatanh fp0",
    " fcos fp0",
    " fcosh fp0",
    " fetox fp0",
    " fetoxm1 fp0",
    " fgetexp fp0",
    " fgetman fp0",
    " flog10 fp0",
    " flog2 fp0",
    " flogn fp0",
    " flognp1 fp0",
    " fmovecr #0,fp0",
    " fsin fp0",
    " fsincos fp0,fp1:fp2",
    " fsinh fp0",
    " ftan fp0",
    " ftanh fp0",
    " ftentox fp0",
    " ftwotox fp0",
    " fmod fp0,fp1",
    " frem fp0,fp1",
    " fscale fp0,fp1",
    " ftrapeq",
    " ftrapne.w #1",
    " fdbeq d0,*",
    " fseq d0",
    " fsne (a0)",
    " fmovem.x d1,-(a0)",
    " fmovem.x (a0)+,d1",
    " fmovem.l #1,fpcr/fpsr",
    " fadd.x #1.5,fp0",
    " fmove.p #1,fp0",
    " fcmp.x #1,fp0",
];

/// Near misses: implemented in hardware.
const NOT_TRAPPED: &[&str] = &[
    " divu.l d0,d1",
    " divs.l (a0),d1",
    " muls.l d0,d1",
    " divsl d0,d1:d2",
    " divul.l d0,d1:d2",
    " divu d0,d1",
    " cas.w d0,d1,(a0)",
    " chk d0,d1",
    " cmp.l d0,d1",
    " fadd fp0,fp1",
    " fmove.s #1.5,fp0",
    " fmove.d #1.5,fp0",
    " fmove.l #1,fp0",
    " fsqrt fp0",
    " fint fp0",
    " fabs fp0",
    " fbeq *+4",
    " fnop",
    " fmovem.x fp0-fp3,-(a0)",
    " fmovem.l #1,fpcr",
    " fmovem.l d0,fpcr",
    " nop",
];

#[test]
fn test_trapped_instructions_warn_on_68060() {
    for source in TRAPPED {
        assert!(warns(source, Cpu::M68060), "{source}");
    }
}

#[test]
fn test_implemented_instructions_do_not_warn_on_68060() {
    for source in NOT_TRAPPED {
        assert!(!warns(source, Cpu::M68060), "{source}");
    }
}

#[test]
fn test_trapped_instructions_do_not_warn_on_other_cpus() {
    for source in TRAPPED {
        assert!(!warns(source, Cpu::M68040), "{source}");
    }
}

#[test]
fn test_trap_warning_follows_machine_directive() {
    let assembly = ours(
        " movep.w d0,4(a0)\n mc68060\n movep.w d0,4(a0)",
        Cpu::M68040,
    );
    let lines: Vec<_> = assembly
        .diagnostics
        .iter()
        .filter_map(|d| d.span.map(|s| s.line))
        .collect();
    assert_eq!(lines, [3]);
}

#[test]
fn test_trap_warning_does_not_stop_output() {
    let assembly = ours(" movep.w d0,4(a0)", Cpu::M68060);
    assert_eq!(assembly.sections[0].data, [0x01, 0x88, 0x00, 0x04]);
}
