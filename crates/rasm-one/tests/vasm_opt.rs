//! Compares optimisation rules with vasm, one rule at a time.

use std::error::Error;

use rasm_one::{
    assemble, write_bin, write_hunk_exe, write_hunk_object, Assembly, Cpu, FpuChoice,
    MemoryProvider, Optimisations, Options, OutputError,
};
use rasm_oracle::{Format, Vasm};

const CPUS: &[&str] = &["68000", "68010", "68020", "68030", "68040", "68060"];
const FORMATS: &[Format] = &[Format::Bin, Format::HunkObject, Format::HunkExe];

/// The rule's own `-opt-` flags for vasm, and what they mean for rasm-one.
struct Rule {
    number: u32,
    flag: &'static str,
}

const BRANCHES: Rule = Rule {
    number: 1,
    flag: "-opt-o1+",
};

fn optimisations(rule: &Rule) -> Optimisations {
    let mut optimisations = Optimisations::none();
    optimisations.set_rule(rule.number, true);
    optimisations
}

fn ours(
    source: &str,
    cpu: &str,
    format: Format,
    rule: &Rule,
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
    options.fixed_addresses = matches!(format, Format::Bin);
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
    rule: &Rule,
    coprocessors: bool,
) -> Result<Option<String>, Box<dyn Error>> {
    let mut args = vec!["-opt-o-", rule.flag];
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

fn compare_rule(sources: &[String], rule: &Rule, coprocessors: bool) -> Result<(), Box<dyn Error>> {
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

const DISPLACEMENTS: Rule = Rule {
    number: 2,
    flag: "-opt-o2+",
};

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
    " section s,bss\nb: ds.w 1\n section c,code\n move.w b(a0),d0",
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

/// In a raw binary a label at address 0 is a displacement of 0. In a hunk file it's a
/// word relocation, which is #51.
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
