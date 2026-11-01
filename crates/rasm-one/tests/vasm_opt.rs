//! Compares optimisation rules with vasm, one rule at a time.

use std::error::Error;

use rasm_one::{
    assemble, write_bin, write_hunk_exe, write_hunk_object, Assembly, Cpu, FpuChoice,
    MemoryProvider, Optimisations, Options, OutputError, Rule, Target,
};
use rasm_oracle::{Format, Vasm};

const CPUS: &[&str] = &["68000", "68010", "68020", "68030", "68040", "68060"];
const FORMATS: &[Format] = &[Format::Bin, Format::HunkObject, Format::HunkExe];

/// A rule, by its flag for vasm.
struct RuleFlag {
    flag: &'static str,
}

const BRANCHES: RuleFlag = RuleFlag { flag: "-opt-o1+" };

fn optimisations(rule: &RuleFlag) -> Optimisations {
    if rule.flag == ALL.flag {
        return Optimisations::all();
    }
    let mut optimisations = Optimisations::none();
    let name = rule.flag.trim_start_matches("-opt-o").trim_end_matches('+');
    if let Some(named) = Rule::named(name) {
        optimisations.set_rule(named, true);
    }
    optimisations
}

fn ours(
    source: &str,
    cpu: &str,
    format: Format,
    rule: &RuleFlag,
    coprocessors: bool,
) -> Result<Vec<u8>, OutputError> {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::new("t.s");
    options.cpu = cpu
        .parse()
        .ok()
        .and_then(Cpu::from_number)
        .unwrap_or_default();
    options.optimisations = optimisations(rule);
    options.target = if matches!(format, Format::Bin) {
        Target::RawBinary
    } else {
        Target::Relocatable
    };
    if coprocessors {
        options.fpu = FpuChoice::Coprocessor;
        options.mmu_68851 = true;
    }
    let assembly: Assembly = assemble(&options, &files);
    match format {
        Format::Bin => write_bin(&assembly),
        Format::HunkObject => write_hunk_object(&assembly),
        Format::HunkExe => write_hunk_exe(&assembly),
    }
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Assembles with both, returning a description of any difference.
fn compare(
    vasm: &Vasm,
    source: &str,
    cpu: &str,
    format: Format,
    rule: &RuleFlag,
    coprocessors: bool,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut args = if rule.flag == ALL.flag {
        vec![rule.flag]
    } else {
        vec!["-opt-o-", rule.flag]
    };
    if coprocessors {
        args.extend(["-m68881", "-m68851"]);
    }
    let theirs = vasm.assemble_with_args(source, cpu, format, &args)?;
    let mine = ours(source, cpu, format, rule, coprocessors);
    let same = match &mine {
        Ok(bytes) => theirs.success && *bytes == theirs.bytes,
        Err(_) => !theirs.success,
    };
    Ok((!same).then(|| {
        let mine = match &mine {
            Ok(bytes) => hex(bytes),
            Err(error) => format!("{error:?}"),
        };
        format!(
            "{cpu} {format:?} {source:?}\n  vasm ({}): {}\n  ours: {mine}\n  {}",
            theirs.success,
            hex(&theirs.bytes),
            theirs.stderr
        )
    }))
}

fn compare_all(sources: &[String], coprocessors: bool) -> Result<(), Box<dyn Error>> {
    compare_rule(sources, &BRANCHES, coprocessors)
}

fn compare_rule(
    sources: &[String],
    rule: &RuleFlag,
    coprocessors: bool,
) -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    // A 68851 needs a 68020 or 68030 in rasm-one.
    let cpus = if coprocessors { &CPUS[2..4] } else { CPUS };
    let mut failures = Vec::new();
    for source in sources {
        for cpu in cpus {
            for format in FORMATS {
                let found = compare(&vasm, source, cpu, *format, rule, coprocessors)?;
                failures.extend(found);
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

const BRANCH_CASES: &[&str] = &[
    // Distances.
    " bra f\n nop\nf:",
    "f: bra f",
    " nop\n bra f\nf: nop",
    " bge f\n nop\nf:",
    " bsr f\n nop\nf:",
    " bra f\n ds.b 124\nf:",
    " bra f\n ds.b 126\nf:",
    " bra f\n ds.b 127\nf:",
    " bra f\n ds.b 128\nf:",
    "f: ds.b 124\n bra f",
    "f: ds.b 126\n bra f",
    "f: ds.b 128\n bra f",
    " bra f\n ds.b 32764\nf:",
    " bra f\n ds.b 32765\nf:",
    " bra f\n ds.b 32766\nf:",
    " bra f\n ds.b 32768\nf:",
    "f: ds.b 32764\n bra f",
    "f: ds.b 32766\n bra f",
    "f: ds.b 32768\n bra f",
    "f: ds.b 32770\n bra f",
    // Too far for a word.
    " bra f\n ds.b 40000\nf:",
    " bsr f\n ds.b 40000\nf:",
    " beq f\n ds.b 40000\nf:",
    " bhi f\n ds.b 40000\nf:",
    " bcc f\n ds.b 40000\nf:",
    " blt f\n ds.b 40000\nf:",
    "f: ds.b 40000\n bra f",
    "f: ds.b 40000\n bne f",
    " opt p+\n bra f\n ds.b 40000\nf:",
    " opt p+\n beq f\n ds.b 40000\nf:",
    // To what follows, which disappears.
    " bra f\nf:",
    " beq f\nf:",
    " bsr f\nf:",
    " bra f\nf: equ *",
    " rept 3\n bra f\n endr\nf:",
    " bra f\n bra f\nf:",
    " bra a\n bra b\n bra c\na:\nb:\nc:",
    "l: bra f\nf: dc.l l,f",
    "l: bra f\n ; c\nf: rts",
    // Depending on themselves or each other.
    "a: bra c\n ds.b 124\n bra a\nc: nop",
    "b1: bra b2\n ds.b 124\nb2: bra b1",
    "a: bra e\n ds.b 60\nb: bra d\n ds.b 60\nc: nop\nd: nop\n ds.b 2\ne: nop",
    " bra a\nb: ds.b 122\n bra b\n nop\na:",
    " bra a\n ds.b 124\n bra c\na: nop\n ds.b 124\nc:",
    " bra g\n bra f\n ds.b 122\nf: nop\n ds.b 2\ng:",
    "s: bra e\ne: ds.b e-s",
    "s: bra e\n ds.b 130-(e-s)\ne:",
    " cnop 0,4\n bra f\n cnop 0,4\nf:",
    " dc.w 0\n bra f\n cnop 0,4\nf:",
    // Around odd addresses.
    " bra *+3",
    " bra *+4",
    " bra.w *+1",
    " bra f\n dc.b 1\nf:",
    " bra f\n dc.b 1\n even\nf:",
    // Displacements of 0, which are reserved in a short branch.
    " bra *+2",
    " bsr *+2",
    " beq *+2",
    " nop\n bra *+2",
    // With a size, and other kinds of branch, nothing changes.
    " bra.w f\nf:",
    " bra.s f\n nop\nf:",
    " bra.s f\nf:",
    " bra.b f\nf:",
    " bsr.b f\nf:",
    " bra.w f\n ds.b 40000\nf:",
    " bra.s f\n ds.b 200\nf:",
    " dbra d0,f\nf:",
    " dbra d0,f\n ds.b 40000\nf:",
    " dbne d0,f\n ds.b 40000\nf:",
    // Somewhere else.
    " xref x\n bra x",
    " xref x\n beq x",
    " xref x\n bsr x",
    " section a,code\n bra f\n section b,code\nf: nop",
    " section a,code\n beq f\n section b,code\nf: nop",
    " section a,code\n bsr f\n section b,code\nf: nop",
    // Switching in the source.
    " opt o1-\n bra f\n nop\nf:",
    " opt o1+\n bra f\n nop\nf:",
    " opt o1\n bra f\n nop\nf:",
    " opt o-\n bra f\n nop\nf:",
    " opt o+\n bra f\n nop\nf:",
    " opt o\n bra f\n nop\nf:",
    " opt o1-\n bra f\n opt o1+\n bra f\n nop\nf:",
];

#[test]
fn test_assemble_branches_match_vasm_o1() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = BRANCH_CASES.iter().map(|s| s.to_string()).collect();
    compare_all(&sources, false)
}

/// Each branch here can only shrink after the one after it has, so this takes far more
/// passes than a small program.
#[test]
fn test_assemble_long_chain_of_shrinking_branches_matches_vasm_o1() -> Result<(), Box<dyn Error>> {
    let mut source = String::new();
    for index in 0..3000 {
        source += &format!("l{index}:\n ds.b 60\n bra l{}\n", index + 2);
    }
    source += "l3000:\nl3001:\n";
    compare_all(&[source], false)
}

/// rasm-one doesn't write `org` to hunk files, so these are for raw binaries.
#[test]
fn test_assemble_org_branches_match_vasm_o1() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    for source in [
        " org $1000\n bra $1002",
        " org $1000\n bra $1004",
        " org $1000\n bra $20000",
        " org $1000\n bsr $20000",
        " org $1000\n bne $20000",
        " org $1000\nf: bra f+4",
        " org $1000\n bra f\n nop\nf:",
        " org $1000\n bra f\n bra f\nf:",
    ] {
        for cpu in CPUS {
            let found = compare(&vasm, source, cpu, Format::Bin, &BRANCHES, false)?;
            assert_eq!(found, None);
        }
    }
    Ok(())
}

#[test]
fn test_assemble_coprocessor_branches_match_vasm_o1() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = [
        " fbeq f\nf:",
        " fbeq f\n nop\nf:",
        " fbeq f\n ds.b 40000\nf:",
        "f: ds.b 40000\n fbne f",
        " fbeq.w f\n ds.b 40000\nf:",
        " fbeq.l f\n nop\nf:",
        " xref x\n fbeq x",
        " fdbeq d0,f\nf:",
        " pbbs f\nf:",
        " pbbs f\n ds.b 40000\nf:",
        " pbbs.w f\n ds.b 40000\nf:",
        " opt o1-\n fbeq f\nf:",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    compare_all(&sources, true)
}

const DISPLACEMENTS: RuleFlag = RuleFlag { flag: "-opt-o2+" };

const DISPLACEMENT_CASES: &[&str] = &[
    " move.w 0(a0),d0",
    " move.w (0,a0),d0",
    " move.w 0(a0),0(a1)",
    " move.w 2(a0),0(a1)",
    " move.w 0(a0),2(a1)",
    " move.l #1,0(a0)",
    " move.w z(a0),d0\nz equ 0",
    " move.w z(a0),d0\n nop\nz equ 0",
    "z equ 0\n move.w z(a0),d0",
    "a: nop\nb: move.w b-a-2(a0),d0",
    "a: move.w b-c(a0),d0\nc: nop\nb:",
    " xref x\n move.w x(a0),d0",
    " jmp 0(a0)",
    " jsr 0(a0)",
    " lea 0(a0),a1",
    " pea 0(a0)",
    " movem.l d0/d1,0(a0)",
    " movem.l 0(a0),d0/d1",
    " clr.w 0(a0)",
    " tst.l 0(a7)",
    " btst #1,0(a0)",
    " bset d1,0(a0)",
    " addq.w #1,0(a0)",
    " asl.w 0(a0)",
    " movep.w 0(a0),d0",
    " movep.l d0,0(a0)",
    " move.w (0.w,a0),d0",
    " move.w 0(a0,d1.w),d0",
    " move.w 0(pc),d0",
    " lea (100000,a0),a1",
    " opt o2-\n move.w 0(a0),d0",
    " opt o2+\n move.w 0(a0),d0",
    " opt o2-\n move.w 0(a0),d0\n opt o2+\n move.w 0(a0),d0",
    " bfextu 0(a0){1:2},d0",
    " bftst 0(a0){1:2}",
    " bfins d0,0(a0){1:2}",
    " mulu.l 0(a0),d0",
    " cas.w d0,d1,0(a0)",
    " chk2.w 0(a0),d0",
    " moves.w 0(a0),d0",
    " moves.w d0,0(a0)",
];

#[test]
fn test_assemble_displacements_match_vasm_o2() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = DISPLACEMENT_CASES.iter().map(|s| s.to_string()).collect();
    compare_rule(&sources, &DISPLACEMENTS, false)
}

/// In a raw binary a label at address 0 is a displacement of 0.
#[test]
fn test_assemble_label_displacements_match_vasm_o2_in_binary() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    for source in [
        "a: nop\n move.w a(a0),d0",
        "a: move.w a(a0),d0",
        "b: nop\na: move.w a(a0),d0",
    ] {
        for cpu in CPUS {
            let found = compare(&vasm, source, cpu, Format::Bin, &DISPLACEMENTS, false)?;
            assert_eq!(found, None);
        }
    }
    Ok(())
}

#[test]
fn test_assemble_coprocessor_displacements_match_vasm_o2() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = [
        " fmove.s 0(a0),fp0",
        " fmove.s fp0,0(a0)",
        " fmove.p fp0,0(a0){#1}",
        " fmovem.x fp0,0(a0)",
        " fmove.l 0(a0),fpcr",
        " fsave 0(a0)",
        " frestore 0(a0)",
        " pmove tc,0(a0)",
        " pflush #1,#1,0(a0)",
        " ptestr #1,0(a0),#1",
        " pload #1,#1,0(a0)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    compare_rule(&sources, &DISPLACEMENTS, true)
}

const ABSOLUTES: RuleFlag = RuleFlag { flag: "-opt-o3+" };

const ABSOLUTE_CASES: &[&str] = &[
    " move.w 4,d0",
    " move.l 4,a6",
    " move.w 4.l,d0",
    " move.w (4).l,d0",
    " move.w 4.w,d0",
    " move.w $7fff,d0",
    " move.w $8000,d0",
    " move.w $ffff,d0",
    " move.w $ffff8000,d0",
    " move.w $ffff7fff,d0",
    " move.w -4,d0",
    " move.w z,d0\nz equ 4",
    " move.w z,d0\nz equ 100000",
    "z equ 4\n move.w z,d0",
    " jmp 4",
    " jsr 4",
    " lea 4,a0",
    " pea 4",
    " move.w d0,4",
    " move.w 4,6",
    " move.w 4,$10000",
    " move.w $10000,6",
    " clr.l 4",
    " movem.l 4,d0/d1",
    " movem.l d0/d1,4",
    " movem.l 4,d0",
    " btst #1,4",
    " addq.w #1,4",
    " xref x\n move.w x,d0",
    " xref x\n jmp x",
    " section a,code\n jmp f\n section b,code\nf: nop",
    " section a,code\nf: move.w f,d0",
    " opt p+\n move.w 4,d0",
    " opt p+\n jmp 4",
    " opt o3-\n move.w 4,d0",
    " opt o3+\n move.w 4,d0",
    " opt o3-\n move.w 4,d0\n opt o3+\n move.w 4,d0",
    " bfextu 4{1:2},d0",
    " bfextu $10000{1:2},d0",
    " move16 (a0)+,4",
    " move16 4,(a1)",
    " dbra d0,*",
    " bra.w *",
    " movem.l 4,a0",
];

#[test]
fn test_assemble_absolutes_match_vasm_o3() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = ABSOLUTE_CASES.iter().map(|s| s.to_string()).collect();
    compare_rule(&sources, &ABSOLUTES, false)
}

/// In a raw binary a label is a final address.
#[test]
fn test_assemble_absolute_labels_match_vasm_o3_in_binary() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    for source in [
        "f: jmp f",
        " jmp f\nf:",
        " move.w f,d0\nf:",
        " jmp f\n ds.b 32762\nf:",
        " jmp f\n ds.b 32764\nf:",
        " jmp f\n ds.b 32766\nf:",
        " jmp f\n jmp g\n ds.b 32758\nf: nop\ng:",
        " jmp g\n jmp f\n ds.b 32758\nf: nop\ng:",
        " jmp f\n jmp f\n ds.b 32760\nf:",
        " org $100\n move.w 4,d0",
        " org $100\nf: move.w f,d0",
        " org $100000\nf: move.w f,d0",
    ] {
        for cpu in CPUS {
            let found = compare(&vasm, source, cpu, Format::Bin, &ABSOLUTES, false)?;
            assert_eq!(found, None);
        }
    }
    Ok(())
}

#[test]
fn test_assemble_coprocessor_absolutes_match_vasm_o3() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = [
        " fmove.s 4,fp0",
        " fmove.s fp0,4",
        " fmove.p fp0,4{#1}",
        " fmovem.x fp0,4",
        " fmovem.l 4,fpcr",
        " fsave 4",
        " pmove tc,4",
        " pflush #1,#1,4",
        " fbeq *",
        " fdbeq d0,*",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    compare_rule(&sources, &ABSOLUTES, true)
}

const QUICK_MOVES: RuleFlag = RuleFlag { flag: "-opt-o4+" };

#[test]
fn test_assemble_quick_moves_match_vasm_o4() -> Result<(), Box<dyn Error>> {
    let mut sources: Vec<String> = [
        " move.w #1,d0",
        " move.b #1,d0",
        " move #1,d0",
        " move.l #1,a0",
        " move.l #1,(a0)",
        " move.l d1,d0",
        " move.l #1,d7",
        " move.l #$10000,d7",
        " move.l #2000,d7",
        " move.l #254,d7",
        " move.l #z,d0\nz equ 5",
        " move.l #z,d0\nz equ 500",
        "z equ 5\n move.l #z,d0",
        " move.l #g-f,d0\nf: nop\ng:",
        " xref x\n move.l #x,d0",
        " section a,code\nf: move.l #f,d0",
        " opt o4-\n move.l #1,d0",
        " opt o4+\n move.l #1,d0",
        " opt o4-\n move.l #1,d0\n opt o4+\n move.l #1,d0",
        " opt p=68040\n move.l #254,d0",
        " opt p=68040\n move.l #1,d0",
        " opt p=68040\n move.l #254,d0\n opt p=68060\n move.l #254,d0",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for value in (-300..300)
        .step_by(7)
        .chain(-260..-250)
        .chain(120..132)
        .chain(250..260)
    {
        sources.push(format!(" move.l #{value},d0"));
    }
    for value in [
        0x7FFF_FFFFu32,
        0x8000_0000,
        0xFFFF_FFFF,
        0xFFFF_FFFE,
        0xFF00,
        0xFFFF,
        0x8000,
        0x1_0000,
        0x1_0001,
        0x7F_0000,
        0x80_0000,
        0x1FE_0000,
        0xFF80_0000,
        0xFFFF_0000,
        0xFFFE_0000,
        0xFFFE_FFFF,
        0xFFFD_FFFF,
        0xFF80_FFFF,
        0xFF7F_FFFF,
        0xFF_0000,
        0x100_0000,
        0x1234_0000,
    ] {
        sources.push(format!(" move.l #{value},d0"));
    }
    compare_rule(&sources, &QUICK_MOVES, false)
}

#[test]
fn test_assemble_quick_move_labels_match_vasm_o4_in_binary() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    for source in [
        " move.l #f,d0\nf:",
        " move.l #f,d0\n ds.b 120\nf:",
        " move.l #f,d0\n ds.b 122\nf:",
        " move.l #f,d0\n ds.b 124\nf:",
        " move.l #f,d0\n ds.b 126\nf:",
        " move.l #f,d0\n ds.b 248\nf:",
        " move.l #f,d0\n ds.b 250\nf:",
        " move.l #f,d0\n ds.b 252\nf:",
        " move.l #f,d0\n dc.b 1\nf:",
        "f: move.l #f,d0",
        " org $100\nf: move.l #f,d0",
        " org $10000\nf: move.l #f,d0",
    ] {
        for cpu in CPUS {
            let found = compare(&vasm, source, cpu, Format::Bin, &QUICK_MOVES, false)?;
            assert_eq!(found, None);
        }
    }
    Ok(())
}

const QUICK_ARITHMETIC: RuleFlag = RuleFlag { flag: "-opt-o5+" };

#[test]
fn test_assemble_quick_arithmetic_matches_vasm_o5() -> Result<(), Box<dyn Error>> {
    let mut sources: Vec<String> = Vec::new();
    for mnemonic in ["add", "adda", "addi", "sub", "suba", "subi"] {
        for size in ["", ".b", ".w", ".l"] {
            for value in ["0", "1", "8", "9", "-1", "$ffffffff", "z", "w"] {
                for destination in [
                    "d0", "a0", "(a0)", "(a0)+", "-(a0)", "4(a0)", "4", "4.l", "sr",
                ] {
                    sources.push(format!(
                        " {mnemonic}{size} #{value},{destination}\nz equ 3\nw equ 300"
                    ));
                }
            }
        }
    }
    sources.push(" xref x\n add.w #x,d0".to_string());
    sources.push(" section a,code\nf: add.l #f,d0".to_string());
    sources.push(" opt o5-\n add.w #1,d0\n opt o5+\n add.w #1,d0".to_string());
    sources.push(" add.w #g-f,d0\nf: nop\ng:".to_string());
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in &sources {
        failures.extend(compare(
            &vasm,
            source,
            "68000",
            Format::Bin,
            &QUICK_ARITHMETIC,
            false,
        )?);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

const SHORT_NEXT: RuleFlag = RuleFlag { flag: "-opt-o7+" };

#[test]
fn test_assemble_short_branches_to_next_match_vasm_o7() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = [
        " bra.b f\nf:",
        " bra.s f\nf:",
        " beq.s f\nf:",
        " beq.b f\nf: nop",
        " bsr.s f\nf:",
        " bra.s *+2",
        " bra.s f\n nop\nf:",
        " bra.w f\nf:",
        " bra f\nf:",
        " dbra d0,*+2",
        " opt o7-\n bra.s f\nf:",
        " opt o7+\n bra.s f\nf:",
        " xref x\n bra.s x",
        " section a,code\n bra.s f\n section b,code\nf: nop",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    compare_rule(&sources, &SHORT_NEXT, false)
}

const NULL_BASE: RuleFlag = RuleFlag { flag: "-opt-o8+" };

const NULL_OUTER: RuleFlag = RuleFlag { flag: "-opt-o9+" };

#[test]
fn test_assemble_null_displacements_match_vasm_o8_o9() -> Result<(), Box<dyn Error>> {
    let mut sources: Vec<String> = Vec::new();
    for base in ["0", "0.w", "0.l", "4", "$8000", "z", "w", "g-f"] {
        for outer in ["0", "0.w", "0.l", "4", "z", "w", "g-f"] {
            for form in [
                format!("([{base},a0],{outer})"),
                format!("([{base},a0,d1.w],{outer})"),
                format!("([{base},a0],d1.w,{outer})"),
                format!("([{base},pc],{outer})"),
                format!("([{base},d1.w],{outer})"),
            ] {
                sources.push(format!(" move.w {form},d0\nz equ 0\nw equ 300\nf: nop\ng:"));
            }
        }
    }
    for form in [
        "(0.w,a0,d1.w)",
        "(0.l,a0,d1.w)",
        "(0,d1.w)",
        "(0.l,a0)",
        "(0.l,pc)",
        "([a0])",
        "([a0],0)",
        "([0,a0])",
        "(1000,a0,d1.w)",
        "([0],0)",
    ] {
        sources.push(format!(" move.w {form},d0"));
    }
    sources.push(" xref x\n move.w ([x,a0],x),d0".to_string());
    sources.push(" opt o8-\n opt o9-\n move.w ([0,a0],0),d0".to_string());
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in &sources {
        for rule in [&NULL_BASE, &NULL_OUTER] {
            failures.extend(compare(&vasm, source, "68020", Format::Bin, rule, false)?);
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

const ADD_TO_LEA: RuleFlag = RuleFlag { flag: "-opt-o10+" };

const LEA_TO_QUICK: RuleFlag = RuleFlag { flag: "-opt-o11+" };

const WORD_IMMEDIATE: RuleFlag = RuleFlag { flag: "-opt-o12+" };

const IMMEDIATES: &[&str] = &[
    "0",
    "1",
    "8",
    "9",
    "-1",
    "-8",
    "-9",
    "127",
    "128",
    "$7fff",
    "$8000",
    "$ffff",
    "$ffff8000",
    "$ffff7fff",
    "$10000",
    "z",
    "w",
    "g-f",
];

fn compare_each(rule: &RuleFlag, sources: &[String]) -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in sources {
        failures.extend(compare(&vasm, source, "68000", Format::Bin, rule, false)?);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

/// Wrong in vasm, so checked on their own below: see vasm-bugs.md.
fn wrong_in_vasm(mnemonic: &str, value: &str, destination: &str) -> bool {
    matches!(mnemonic, "sub" | "suba") && value == "$8000" && destination != "d0"
}

#[test]
fn test_assemble_address_arithmetic_matches_vasm_o10() -> Result<(), Box<dyn Error>> {
    let mut sources = Vec::new();
    for mnemonic in ["add", "adda", "addi", "sub", "suba", "subi", "cmp", "move"] {
        for size in [".w", ".l", ""] {
            for value in IMMEDIATES {
                for destination in ["a0", "sp", "d0"] {
                    // vasm takes `movea #1,d0` as a `move`, but rasm-one doesn't.
                    if mnemonic.ends_with('a') && destination == "d0" {
                        continue;
                    }
                    let line = format!("{mnemonic}{size} #{value},{destination}");
                    if !wrong_in_vasm(mnemonic, value, destination) {
                        sources.push(format!(" {line}\nz equ 3\nw equ 300\nf: nop\ng:"));
                    }
                }
            }
        }
    }
    sources.push(" opt o10-\n add.w #300,a0".to_string());
    compare_each(&ADD_TO_LEA, &sources)
}

#[test]
fn test_assemble_small_lea_matches_vasm_o11() -> Result<(), Box<dyn Error>> {
    let mut sources = Vec::new();
    for value in IMMEDIATES {
        for form in [
            "{v}(a0),a0",
            "{v}(a0),a1",
            "{v}(sp),sp",
            "({v}.w,a0),a0",
            "({v}.l,a0),a0",
            "({v},a0,d0.w),a0",
        ] {
            let operands = form.replace("{v}", value);
            sources.push(format!(" lea {operands}\nz equ 3\nw equ 300\nf: nop\ng:"));
        }
    }
    sources.push(" lea (a0),a0".to_string());
    sources.push(" xref x\n lea x(a0),a0".to_string());
    sources.push(" opt o11-\n lea 4(a0),a0".to_string());
    compare_each(&LEA_TO_QUICK, &sources)
}

#[test]
fn test_assemble_word_immediates_match_vasm_o12() -> Result<(), Box<dyn Error>> {
    let mut sources = Vec::new();
    for mnemonic in [
        "add", "adda", "sub", "suba", "cmp", "cmpa", "move", "movea", "addi", "and",
    ] {
        for size in [".l", ".w"] {
            for value in IMMEDIATES {
                for destination in ["a0", "d0"] {
                    // vasm takes `movea #1,d0` as a `move`, but rasm-one doesn't.
                    if mnemonic.ends_with('a') && destination == "d0" {
                        continue;
                    }
                    let line = format!("{mnemonic}{size} #{value},{destination}");
                    if !wrong_in_vasm(mnemonic, value, destination) {
                        sources.push(format!(" {line}\nz equ 3\nw equ 300\nf: nop\ng:"));
                    }
                }
            }
        }
    }
    sources.push(" xref x\n move.l #x,a0".to_string());
    sources.push(" opt o12-\n move.l #1,a0".to_string());
    compare_each(&WORD_IMMEDIATE, &sources)
}

/// Each of these is a different calculation as vasm optimises it, so stays as written.
#[test]
fn test_assemble_subtracting_8000_from_address_register_stays_correct() {
    for (line, expected, rule) in [
        (
            " sub.w #$8000,a0",
            vec![0x90, 0xFC, 0x80, 0x00],
            &ADD_TO_LEA,
        ),
        (
            " sub.l #$8000,a0",
            vec![0x41, 0xE8, 0x80, 0x00],
            &ADD_TO_LEA,
        ),
        (
            " sub.l #$8000,a0",
            vec![0x91, 0xFC, 0x00, 0x00, 0x80, 0x00],
            &WORD_IMMEDIATE,
        ),
    ] {
        let bytes = ours(line, "68000", Format::Bin, rule, false);
        assert_eq!(bytes.ok(), Some(expected), "{line}");
    }
}

const GENERIC: RuleFlag = RuleFlag { flag: "-opt-og+" };

fn generic_cases() -> Vec<String> {
    let mut sources: Vec<String> = [
        " lea 0,a0",
        " lea 0.w,a0",
        " lea 0.l,a0",
        " lea 4,a0",
        " lea 0(a0),a0",
        " lea (0.w,a0),a0",
        " lea (a0),a0",
        " lea (a0),a1",
        " lea 0(a0),a1",
        " lea z(a0),a0",
        " lea 100000(a0),a0",
        " lea 100000(a0),a1",
        " lea -100000(sp),sp",
        " lea 32768(a0),a1",
        " lea 32767(a0),a1",
        " movem.l (a0),a1",
        " movem.w (a0),a1",
        " movem.l 4(a0),a1",
        " movem.l (a0)+,a1",
        " movem.l (a0),d1",
        " movem.l d1,(a0)",
        " movem.l a1,(a0)",
        " movem.l (a0),d0/d1",
        " movem.l (a0),a1/a2",
        " movem.l #0,(a0)",
        " movem.l (a0),#0",
        " movem.l #0,-(a0)",
        " movem.w #0,(a0)",
        " movem.l (a0),#$200",
        " move.l #f,a0\nf:",
        " movea.l #f,a0\nf:",
        " move.w #f,a0\nf:",
        " xref x\n move.l #x,a0",
        " clr.l d0",
        " clr.l d7",
        " clr.w d0",
        " clr.b d0",
        " clr.l (a0)",
        " link a6,#0",
        " link.w a6,#-4",
        " link.l a6,#0",
        " link.l a6,#-4",
        " link.l a6,#-32769",
        " link.l a6,#z",
        " asl.w d0",
        " asl d0",
        " asl.w (a0)",
        " opt og-\n clr.l d0",
        " opt og+\n clr.l d0",
        " tst.l d0",
        " movem.l (sp)+,a1/a2",
        " movem.w (a0)+,a2/a1",
        " movem.l (a0)+,a0/a1",
        " movem.l (a1)+,a0/a1",
        " movem.l (a1)+,a0/a7",
        " movem.l (sp)+,a0/a7",
        " movem.l (a0)+,d1/a1",
        " movem.l (a0)+,d1/d2",
        " movem.l (a0)+,a1/a2/a3",
        " movem.l (a0)+,#$600",
        " movem.l (a0),a1/a2",
        " movem.l a1/a2,-(sp)",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for mnemonic in [
        "cmp", "cmpi", "cmpa", "move", "movea", "and", "andi", "or", "ori", "eor", "eori", "add",
        "adda", "addi", "sub", "suba", "subi",
    ] {
        for size in [".b", ".w", ".l"] {
            for value in ["0", "z", "-1", "1", "$ff", "$ffff", "$ffffffff", "$100"] {
                for destination in ["d0", "(a0)", "4(a0)", "4", "a0", "sp"] {
                    sources.push(format!(" {mnemonic}{size} #{value},{destination}"));
                }
            }
        }
    }
    for mnemonic in ["asl", "asr", "lsl", "lsr", "rol"] {
        for size in [".b", ".w", ".l"] {
            for value in ["1", "2", "z"] {
                sources.push(format!(" {mnemonic}{size} #{value},d3"));
            }
        }
    }
    sources
        .into_iter()
        .map(|source| format!("{source}\nz equ 0"))
        .collect()
}

/// Whether rasm-one rejects this, where vasm accepts it as something else. Some of those
/// are choices, and `eor.b #$ffff` is a vasm bug: see vasm-bugs.md.
fn stricter(source: &str) -> bool {
    if source.contains("eor") && source.contains(".b #$ffff,") {
        return true;
    }
    source.contains("movea") && !source.contains(",a") && !source.contains(",sp")
        || source.contains("cmpa") && !source.contains(",a") && !source.contains(",sp")
        || source.contains("adda") && !source.contains(",a") && !source.contains(",sp")
        || source.contains("suba") && !source.contains(",a") && !source.contains(",sp")
}

#[test]
fn test_assemble_generic_rewrites_match_vasm_og() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in generic_cases().iter().filter(|source| !stricter(source)) {
        for cpu in CPUS {
            failures.extend(compare(
                &vasm,
                source,
                cpu,
                Format::HunkObject,
                &GENERIC,
                false,
            )?);
        }
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

/// vasm accepts this with `og`, as if the value fitted. See vasm-bugs.md.
#[test]
fn test_assemble_eor_byte_out_of_range_stays_an_error() {
    let bytes = ours(" eor.b #$ffff,d0", "68000", Format::Bin, &GENERIC, false);
    assert!(bytes.is_err());
}

/// All rules together, which is `-opt-o+`, against vasm with the same.
const ALL: RuleFlag = RuleFlag { flag: "-opt-o+" };

#[test]
fn test_assemble_pc_relative_matches_vasm_all_rules() -> Result<(), Box<dyn Error>> {
    let sources: Vec<String> = [
        " lea f,a0\nf:",
        " lea f,a0\n nop\nf: nop",
        " lea f.l,a0\nf:",
        " lea f.w,a0\nf:",
        " lea 4,a0",
        " move.l f,d0\nf:",
        " move.l d0,f\nf:",
        " move.l f,f\nf:",
        " move.l f,d0\n ds.b 32760\nf:",
        " move.l f,d0\n ds.b 32766\nf:",
        " move.l f,d0\n ds.b 40000\nf:",
        " tst.l f\nf:",
        " cmp.l f,d0\nf:",
        " add.l f,d0\nf:",
        " add.l d0,f\nf:",
        " clr.l f\nf:",
        " pea f\nf:",
        " jmp f\nf:",
        " jmp f\n nop\nf:",
        " jsr f\n nop\nf:",
        " jsr f\nf:",
        " jmp f\n ds.b 40000\nf:",
        " jsr f\n ds.b 40000\nf:",
        "f: ds.b 40000\n jmp f",
        " jmp 4",
        " jsr 4",
        " jmp $10000",
        " jmp f.l\nf:",
        " jmp (a0)",
        " jmp f(pc)\nf:",
        " section a,code\n lea f,a0\n section b,code\nf: nop",
        " section a,code\n jmp f\n section b,code\nf: nop",
        " section a,code\n jsr f\n section b,code\nf: nop",
        " xref x\n lea x,a0",
        " xref x\n jmp x",
        " xref x\n jsr x",
        " bfextu f{1:2},d0\nf:",
        " btst #1,f\nf:",
        " btst d0,f\nf:",
        " cmpi.l #1,f\nf:",
        " cmp.w f,a0\nf:",
        " chk.w f,d0\nf:",
        " mulu f,d0\nf:",
        " move.w f,ccr\nf:",
        " movem.l f,d0/d1\nf:",
        " move.l #f,a0\nf:",
        " move.l #f,a0\n ds.b 40000\nf:",
        " opt o-\n lea f,a0\nf:",
        " opt o-\n jmp f\nf:",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    compare_rule(&sources, &ALL, false)
}

const FLOAT: RuleFlag = RuleFlag { flag: "-opt-of+" };

#[test]
fn test_assemble_float_constants_match_vasm_of() -> Result<(), Box<dyn Error>> {
    let values = [
        "0",
        "1",
        "-1",
        "1.0",
        "1.5",
        "0.5",
        "-2.5",
        "100",
        "100.0",
        "32767",
        "32768",
        "-32768",
        "-32769",
        "65535",
        "65536",
        "16777216",
        "1e10",
        "2147483648",
        "4.0",
        "4",
        "3.0",
        "-2.0",
        "8",
        "0.25",
        "$10",
        "$7fffffff",
        "z",
    ];
    let mut sources = Vec::new();
    for size in ["s", "d", "x", "b", "w", "l"] {
        for value in values {
            // `fmove.l #1.5` is vasm bug 7.
            if size == "l" && (value.contains('.') || value.contains('e')) {
                continue;
            }
            for form in [
                format!(" fmove.{size} #{value},fp0"),
                format!(" fadd.{size} #{value},fp1"),
                format!(" fmul.{size} #{value},fp0"),
                format!(" fcmp.{size} #{value},fp0"),
                format!(" fdiv.{size} #{value},fp0"),
                format!(" fsdiv.{size} #{value},fp0"),
                format!(" fddiv.{size} #{value},fp0"),
                format!(" fsgldiv.{size} #{value},fp0"),
                format!(" ftst.{size} #{value}"),
                format!(" fsmove.{size} #{value},fp0"),
            ] {
                sources.push(format!("{form}\nz equ 7"));
            }
        }
    }
    sources.push(" fmove #1.0,fp0".to_string());
    sources.push(" fsincos #2,fp2:fp3".to_string());
    sources.push(" fsincos.d #2.0,fp2:fp3".to_string());
    sources.push(" fsincos.x #1.5,fp2:fp3".to_string());
    sources.push(" fmove.s #1.1,fp0".to_string());
    sources.push(" fmove.d #1.1,fp0".to_string());
    sources.push(" fmove.d #0.1,fp0".to_string());
    sources.push(" opt of-\n fmove.d #1.0,fp0".to_string());
    sources.push(" opt of+\n fmove.d #1.0,fp0".to_string());
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in &sources {
        failures.extend(compare(&vasm, source, "68040", Format::Bin, &FLOAT, false)?);
    }
    assert!(
        failures.is_empty(),
        "{} differences:\n{}",
        failures.len(),
        failures.join("\n")
    );
    Ok(())
}

/// vasm goes through a double, so narrowing loses bits. See vasm-bugs.md.
#[test]
fn test_assemble_extended_float_stays_extended_when_it_would_lose_bits() {
    let bytes = ours(" fmove.x #1.1,fp0", "68040", Format::Bin, &FLOAT, false);
    assert_eq!(bytes.ok().map(|bytes| bytes.len()), Some(16));
}

/// vasm gets this wrong. See vasm-bugs.md.
#[test]
fn test_assemble_one_operand_asl_keeps_its_register() {
    let bytes = ours(" asl d1", "68000", Format::Bin, &GENERIC, false);
    assert_eq!(bytes.ok(), Some(vec![0xD2, 0x41]));
}

/// An address in `movem` is memory unless it's in the list's place.
#[test]
fn test_assemble_movem_with_a_long_address_stays_movem() {
    for (line, expected) in [
        (" movem.l d0-d1,0.l", vec![0x48, 0xF9, 0, 3, 0, 0, 0, 0]),
        (" movem.l 0.l,d0-d1", vec![0x4C, 0xF9, 0, 3, 0, 0, 0, 0]),
        (" movem.l d0,$100.l", vec![0x48, 0xF9, 0, 1, 0, 0, 1, 0]),
    ] {
        let bytes = ours(line, "68000", Format::Bin, &ALL, false);
        assert_eq!(bytes.ok(), Some(expected), "{line}");
    }
}

/// A list can only hold 16 registers, and vasm refuses or ignores what's above them.
#[test]
fn test_assemble_movem_list_past_16_bits_is_not_a_move() {
    let bytes = ours(" movem.l (a0)+,#$30000", "68000", Format::Bin, &ALL, false);
    assert!(!bytes.is_ok_and(|bytes| bytes.starts_with(&[0x30, 0x58])));
}

/// A small generator, so that runs are the same everywhere.
struct Random(u64);

impl Random {
    fn below(&mut self, limit: usize) -> usize {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
        ((self.0 >> 33) as usize) % limit
    }

    fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
        choices[self.below(choices.len())]
    }
}

/// A program of branches among labels, with gaps near the limits of each form.
fn random_program(random: &mut Random, cnop: bool) -> String {
    const GAPS: &[&str] = &[
        "2", "4", "8", "60", "118", "120", "122", "124", "126", "128", "130", "1000", "40000",
    ];
    const BRANCHES: &[&str] = &["bra", "bsr", "beq", "bhi", "blt"];
    let labels = 2 + random.below(5);
    let mut source = String::new();
    for _ in 0..labels + 2 + random.below(8) {
        let target = random.below(labels);
        match random.below(8) {
            0..=3 => {
                source += &format!(" {} l{target}\n", random.pick(BRANCHES));
            }
            4 => source += &format!(" ds.b {}\n", random.pick(GAPS)),
            5 => source += " nop\n",
            6 if cnop => source += " cnop 0,4\n",
            6 => source += " nop\n nop\n",
            _ => source += " dc.b 1\n",
        }
        if random.below(2) == 0 {
            let label = random.below(labels);
            if !source.contains(&format!("l{label}:")) {
                source += &format!("l{label}:\n");
            }
        }
    }
    for label in 0..labels {
        if !source.contains(&format!("l{label}:")) {
            source += &format!("l{label}:\n");
        }
    }
    source
}

#[test]
fn test_assemble_random_branches_match_vasm_o1() -> Result<(), Box<dyn Error>> {
    let mut random = Random(0x68000);
    let sources: Vec<String> = (0..300)
        .map(|_| random_program(&mut random, false))
        .collect();
    compare_all(&sources, false)
}

/// vasm rejects this, but a word branch reaches, as it does with `.w`. See vasm-bugs.md.
#[test]
fn test_assemble_branch_to_odd_next_byte_uses_word_on_68020() {
    for (source, cpu) in [
        (" bra *+1", "68020"),
        (" dc.b 1\n even\nf: bra f+1", "68060"),
    ] {
        let bytes = ours(source, cpu, Format::Bin, &BRANCHES, false);
        let ok = bytes.is_ok_and(|bytes| bytes.ends_with(&[0x60, 0x00, 0xFF, 0xFF]));
        assert!(ok, "{source}");
    }
}

#[test]
fn test_assemble_far_branch_warns_and_relocates_on_68000() -> Result<(), Box<dyn Error>> {
    let mut files = MemoryProvider::new();
    files.insert("t.s", " beq f\n ds.b 40000\nf: rts");
    let mut options = Options::new("t.s");
    options.optimisations = optimisations(&BRANCHES);
    let assembly = assemble(&options, &files);
    assert!(assembly
        .diagnostics
        .iter()
        .any(|d| d.code == "branch-to-jump"));
    let section = &assembly.sections[0];
    assert_eq!(section.data[..4], [0x66, 0x06, 0x4E, 0xF9]);
    assert_eq!(section.relocs.len(), 1);
    assert_eq!(section.relocs[0].offset, 4);
    Ok(())
}
