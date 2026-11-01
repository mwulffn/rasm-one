//! Sweeps every mnemonic, size and operand class on every CPU against
//! vasm -no-opt.
//!
//! The mnemonic list is independent of the table, and the sweep covers
//! combinations the table calls invalid, so a missing form can't go unnoticed.

use std::collections::HashSet;
use std::error::Error;

use rasm_one::{assemble, Assembly, Cpu, FpuChoice, MemoryProvider, Options};
use rasm_oracle::{Format, Vasm};

/// A machine to sweep: its name for vasm, and whether it has a coprocessor FPU.
struct Machine {
    name: &'static str,
    cpu: Cpu,
    fpu: FpuChoice,
    mmu_68851: bool,
    vasm_args: &'static [&'static str],
}

const fn machine(name: &'static str, cpu: Cpu) -> Machine {
    Machine {
        name,
        cpu,
        fpu: FpuChoice::BuiltIn,
        mmu_68851: false,
        vasm_args: &[],
    }
}

const MACHINES: &[Machine] = &[
    machine("68000", Cpu::M68000),
    machine("68010", Cpu::M68010),
    machine("68020", Cpu::M68020),
    machine("68030", Cpu::M68030),
    machine("68040", Cpu::M68040),
    machine("68060", Cpu::M68060),
    Machine {
        name: "68020",
        cpu: Cpu::M68020,
        fpu: FpuChoice::Coprocessor,
        mmu_68851: false,
        vasm_args: &["-m68881"],
    },
    Machine {
        name: "68030",
        cpu: Cpu::M68030,
        fpu: FpuChoice::Coprocessor,
        mmu_68851: false,
        vasm_args: &["-m68882"],
    },
    Machine {
        name: "68020",
        cpu: Cpu::M68020,
        fpu: FpuChoice::BuiltIn,
        mmu_68851: true,
        vasm_args: &["-m68851"],
    },
];

const MNEMONICS: &[&str] = &[
    "abcd", "add", "adda", "addi", "addq", "addx", "and", "andi", "asl", "asr", "bcc", "bchg",
    "bclr", "bcs", "beq", "bfchg", "bfclr", "bfexts", "bfextu", "bfffo", "bfins", "bfset", "bftst",
    "bge", "bgt", "bhi", "bhs", "bkpt", "ble", "blo", "bls", "blt", "bmi", "bne", "bpl", "bra",
    "bset", "bsr", "btst", "bvc", "bvs", "callm", "cas", "cas2", "chk", "chk2", "cinva", "cinvl",
    "cinvp", "clr", "cmp", "cmp2", "cmpa", "cmpi", "cmpm", "cpusha", "cpushl", "cpushp", "dbcc",
    "dbcs", "dbeq", "dbf", "dbge", "dbgt", "dbhi", "dbhs", "dble", "dblo", "dbls", "dblt", "dbmi",
    "dbne", "dbpl", "dbra", "dbt", "dbvc", "dbvs", "divs", "divsl", "divu", "divul", "eor", "eori",
    "exg", "ext", "extb", "illegal", "jmp", "jsr", "lea", "link", "lpstop", "lsl", "lsr", "move",
    "move16", "movea", "movec", "movem", "movep", "moveq", "moves", "muls", "mulu", "nbcd", "neg",
    "negx", "nop", "not", "or", "ori", "pack", "pea", "reset", "rol", "ror", "roxl", "roxr", "rtd",
    "rte", "rtm", "rtr", "rts", "sbcd", "scc", "scs", "seq", "sf", "sge", "sgt", "shi", "shs",
    "sle", "slo", "sls", "slt", "smi", "sne", "spl", "st", "stop", "sub", "suba", "subi", "subq",
    "subx", "svc", "svs", "swap", "tas", "trap", "trapcc", "trapcs", "trapeq", "trapf", "trapge",
    "trapgt", "traphi", "traphs", "traple", "traplo", "trapls", "traplt", "trapmi", "trapne",
    "trappl", "trapt", "trapv", "trapvc", "trapvs", "tst", "unlk", "unpk",
];

/// A sample of the FPU's mnemonics. The condition codes are tested separately.
const FPU_MNEMONICS: &[&str] = &[
    "fabs", "facos", "fadd", "fasin", "fatan", "fatanh", "fbeq", "fbngl", "fbst", "fcmp", "fcos",
    "fcosh", "fdabs", "fdadd", "fdbeq", "fdbngl", "fddiv", "fdiv", "fdmove", "fdmul", "fdneg",
    "fdsqrt", "fdsub", "fetox", "fetoxm1", "fgetexp", "fgetman", "fint", "fintrz", "flog10",
    "flog2", "flogn", "flognp1", "fmod", "fmove", "fmovecr", "fmovem", "fmul", "fneg", "fnop",
    "frem", "frestore", "fsabs", "fsadd", "fsave", "fscale", "fsdiv", "fseq", "fsgldiv", "fsglmul",
    "fsin", "fsincos", "fsinh", "fsmove", "fsmul", "fsneg", "fsngl", "fsqrt", "fssqrt", "fssub",
    "fsub", "ftan", "ftanh", "ftentox", "ftrapeq", "ftrapngl", "ftst", "ftwotox",
];

/// A sample of the MMU's mnemonics. The condition codes are tested separately.
const MMU_MNEMONICS: &[&str] = &[
    "pbbs", "pbcc", "pdbbs", "pflush", "pflusha", "pflushan", "pflushn", "pflushr", "pflushs",
    "plpar", "plpaw", "ploadr", "ploadw", "pmove", "pmovefd", "prestore", "psave", "psbs",
    "ptestr", "ptestw", "ptrapbs", "pvalid",
];

/// Operands only MMU instructions know. Anywhere else they would be symbols.
const MMU_OPERANDS: &[&str] = &[
    "tc", "srp", "crp", "psr", "mmusr", "drp", "cal", "val", "scc", "ac", "pcsr", "bad0", "bac7",
    "tt0", "tt1", "sfc", "dfc",
];

/// Whole operand lists for MMU forms with several operands.
const MMU_LISTS: &[&str] = &[
    "#1,#2",
    "#1,#2,(a3)",
    "d1,#2",
    "sfc,#2",
    "dfc,#2,6(a6)",
    "#1,#2,(a4)+",
    "#1,(a3)",
    "sfc,(a3)",
    "d1,6(a6)",
    "#1,(a3),#2",
    "d1,6(a6),#2",
    "sfc,(a3),#2,a1",
    "#1,(a4)+,#2",
    "tc,(a3)",
    "(a3),tc",
    "crp,6(a6)",
    "$20.w,srp",
    "tc,d1",
    "d1,tc",
    "tc,a2",
    "(a3)+,tc",
    "tc,-(a5)",
    "*+8(pc),tc",
    "#1,tc",
    "val,(a3)",
    "a1,(a3)",
    "a1,6(a6)",
];

const SIZES: &[&str] = &["", ".b", ".w", ".l", ".s"];
const FPU_SIZES: &[&str] = &[".d", ".x", ".p"];
const MMU_SIZES: &[&str] = &[".q"];

/// Operands only FPU instructions know. Anywhere else they would be symbols.
const FPU_OPERANDS: &[&str] = &[
    "fp1",
    "fp2:fp3",
    "fp0-fp3",
    "fp0/fp2-fp4",
    "fpcr",
    "fpsr",
    "fpiar",
    "fpcr/fpsr",
    "fpcr/fpsr/fpiar",
    "#1.5",
    "(a3){#3}",
    "(a3){d1}",
];

/// One sample per operand class. Addresses stay near `*` to fit any branch.
const OPERANDS: &[&str] = &[
    "d1",
    "a2",
    "(a3)",
    "(a4)+",
    "-(a5)",
    "6(a6)",
    "6(a1,d7.w)",
    "$20.w",
    "*+8",
    "*+8(pc)",
    "*+8(pc,d3.l)",
    "#1",
    "sr",
    "ccr",
    "usp",
    "d0-d2/a0",
];

/// Second operands use different registers, so that swapped register fields
/// show up as different bytes.
const SECOND_OPERANDS: &[&str] = &[
    "d4",
    "a5",
    "(a6)",
    "(a1)+",
    "-(a2)",
    "8(a3)",
    "8(a4,d2.l)",
    "$24.w",
    "*+8",
    "*+8(pc)",
    "*+8(pc,d6.w)",
    "#2",
    "sr",
    "ccr",
    "usp",
    "d3-d5/a4",
];

/// Modes and words used with only a few partners, to keep the sweep's size sane.
const RARE_OPERANDS: &[&str] = &[
    "(100000,a6)",
    "(4.l,a6)",
    "(4.w,a6)",
    "(1000,a1,d7.l*2)",
    "4(a6,d7.w*4)",
    "(4.w,a1,d7)",
    "([4,a6],d7,8)",
    "([4,a6,d7],8)",
    "([a6])",
    "([100000,a6],d7*2,100000)",
    "(d7)",
    "(4,d7.l)",
    "([4])",
    "(a6,zd7)",
    "(za6,d7)",
    "(100000,pc)",
    "(1000,pc,d3*2)",
    "([*+8,pc],d3,8)",
    "d1:d2",
    "(a1):(a2)",
    "(d1):(d2)",
    "d1{1:2}",
    "d1{d2:d3}",
    "(a3){1:d2}",
    "6(a6){d1:2}",
    "$20.w{1:32}",
    "*+8(pc){1:2}",
    "(a4)+{1:2}",
];

/// Control register names, which are only special to `movec`. Anywhere else
/// they are symbols, and vasm reports undefined symbols only if nothing else is
/// wrong, which a batch of lines never achieves.
const CONTROL_REGISTERS: &[&str] = &[
    "sfc", "dfc", "usp", "vbr", "cacr", "caar", "msp", "isp", "tc", "itt0", "itt1", "dtt0", "dtt1",
    "iacr0", "iacr1", "dacr0", "dacr1", "mmusr", "urp", "srp", "buscr", "pcr",
];

/// Cache names, which are only special to `cinv` and `cpush`.
const CACHES: &[&str] = &["nc", "dc", "ic", "bc"];

const RARE_PARTNERS: &[&str] = &["d4", "a5", "(a6)", "(a1)+", "-(a2)", "#2", "$100", "fp5"];

/// Whole operand lists for forms with three operands.
const THREE_OPERANDS: &[&str] = &[
    "d1,d2,(a3)",
    "d1,d2,6(a6)",
    "d1,d2,$20.w",
    "d1,d2,(a4)+",
    "d1,d2,*+8(pc)",
    "d1,d2,#1",
    "-(a1),-(a2),#1",
    "d1:d2,d3:d4,(a1):(a2)",
    "d1:d2,d3:d4,(d1):(a2)",
    "d1:d2,d3:d4,(a1)",
];

/// Valid forms that vasm rejects, listed in docs/vasm-bugs.md.
fn vasm_wrongly_rejects(line: &str) -> bool {
    let head = line.split_whitespace().next().unwrap_or("");
    let mnemonic = head.split('.').next().unwrap_or("");
    let operands = line
        .trim_start()
        .split_once(' ')
        .map_or("", |(_, rest)| rest);
    (matches!(mnemonic, "ploadr" | "ploadw")
        && (operands.starts_with("sfc,") || operands.starts_with("dfc,")))
        || (matches!(mnemonic, "divs" | "divu" | "muls" | "mulu")
            && line.ends_with(" (100000,a6),d4"))
        || (matches!(mnemonic, "jmp" | "jsr") && line.ends_with(" (100000,pc)"))
}

/// Reviewed cases where we deliberately reject what vasm accepts.
fn stricter_than_vasm(line: &str) -> bool {
    // vasm treats `movea` to anything but an address register as `move`. The
    // PRM defines `movea` only for address registers.
    let movea_to_non_address = line.starts_with(" movea") && !line.ends_with(",a5");
    // vasm allows an address as a `movem` mask in raw binaries, where addresses
    // are constants, but not in hunk output. We require a constant everywhere.
    let is_movem = line.starts_with(" movem") || line.starts_with(" fmovem");
    let address_as_mask = is_movem && (line.contains(" *+8,") || line.ends_with(",*+8"));
    // vasm ignores whatever follows `trapcc`, and any size on it, as if it took
    // no operands. Only the plain form and `.w #` or `.l #` are valid.
    let (head, operands) = line
        .trim_start()
        .split_once(' ')
        .unwrap_or((line.trim_start(), ""));
    let (mnemonic, suffix) = head.split_once('.').unwrap_or((head, ""));
    let is_trapcc = (mnemonic.starts_with("trap") && !matches!(mnemonic, "trap" | "trapv"))
        || mnemonic.starts_with("ftrap")
        || mnemonic.starts_with("ptrap");
    let valid_trapcc = match suffix {
        "" => operands.is_empty() || operands == "#1",
        "w" | "l" => operands == "#1",
        _ => false,
    };
    let trapcc_with_junk = is_trapcc && !valid_trapcc;
    // vasm stores the bits of a float as a long here, listed in docs/vasm-bugs.md.
    let float_as_long = head.ends_with(".l") && operands.starts_with("#1.5");
    // vasm ignores `.q` on some forms of these unsized instructions, and refuses
    // it on others. It means nothing on any of them.
    let quad_on_unsized_mmu = head.ends_with(".q")
        && matches!(
            mnemonic,
            "pflush" | "ploadr" | "ploadw" | "ptestr" | "ptestw"
        );
    movea_to_non_address
        || quad_on_unsized_mmu
        || address_as_mask
        || trapcc_with_junk
        || float_as_long
}

fn lines_for(mnemonic: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let is_fpu = FPU_MNEMONICS.contains(&mnemonic);
    let is_mmu = MMU_MNEMONICS.contains(&mnemonic);
    let extra_sizes: &[&str] = match (is_fpu, is_mmu) {
        (true, _) => FPU_SIZES,
        (_, true) => MMU_SIZES,
        _ => &[],
    };
    for size in SIZES.iter().chain(extra_sizes) {
        let head = format!(" {mnemonic}{size}");
        lines.push(head.clone());
        for a in OPERANDS {
            lines.push(format!("{head} {a}"));
            for b in SECOND_OPERANDS {
                lines.push(format!("{head} {a},{b}"));
            }
        }
        let words: &[&str] = match mnemonic {
            "movec" => CONTROL_REGISTERS,
            "cinva" | "cinvl" | "cinvp" | "cpusha" | "cpushl" | "cpushp" => CACHES,
            _ if is_fpu => FPU_OPERANDS,
            _ if is_mmu => MMU_OPERANDS,
            _ => &[],
        };
        for rare in RARE_OPERANDS.iter().chain(words) {
            lines.push(format!("{head} {rare}"));
            for partner in RARE_PARTNERS {
                lines.push(format!("{head} {rare},{partner}"));
                lines.push(format!("{head} {partner},{rare}"));
            }
        }
        for list in THREE_OPERANDS {
            lines.push(format!("{head} {list}"));
        }
        if is_mmu {
            for list in MMU_LISTS {
                lines.push(format!("{head} {list}"));
            }
        }
    }
    lines
}

/// What vasm said about a batch of lines, by line index.
#[derive(Default)]
struct VasmVerdict {
    rejected: HashSet<usize>,
    /// Lines it accepted only by ignoring text it couldn't parse.
    lenient: HashSet<usize>,
}

/// vasm warnings that mean it ignored or guessed at part of a line. The dialect
/// is strict, so we report those as errors.
const LENIENT_WARNINGS: &[&str] = &[
    "warning 2034 ",
    "warning 2068 ",
    "warning 7 ",
    "warning 1004 ",
    "warning 1005 ",
    "warning 1007 ",
    "warning 2007 ",
];

/// The line number in a vasm message, if it has one.
fn message_line(message: &str) -> Option<usize> {
    message
        .split(" in line ")
        .nth(1)
        .and_then(|rest| rest.split_whitespace().next())
        .and_then(|number| number.parse().ok())
}

/// Asks vasm which lines it rejects. It stops at a fatal error, and only reports
/// some errors, such as values out of range, if there were no earlier ones. So
/// it is run on what remains until it has nothing more to say.
fn vasm_verdict(vasm: &Vasm, lines: &[String], machine: &Machine) -> Result<VasmVerdict, String> {
    let cpu = machine.name;
    let mut verdict = VasmVerdict::default();
    loop {
        let remaining: Vec<usize> = (0..lines.len())
            .filter(|index| !verdict.rejected.contains(index))
            .collect();
        let source = remaining
            .iter()
            .map(|&index| format!("{}\n", lines[index]))
            .collect::<Vec<_>>()
            .concat();
        let out = vasm
            .assemble_with_args(&source, cpu, Format::Bin, machine.vasm_args)
            .map_err(|error| error.to_string())?;
        let mut found_errors = false;
        for message in out.stderr.lines() {
            let is_error = message.starts_with("error") || message.starts_with("fatal error");
            let placed = message_line(message).and_then(|number| remaining.get(number - 1));
            if LENIENT_WARNINGS
                .iter()
                .any(|code| message.starts_with(code))
            {
                if let Some(&index) = placed {
                    verdict.lenient.insert(index);
                }
            } else if is_error {
                found_errors = true;
                if let Some(&index) = placed {
                    verdict.rejected.insert(index);
                } else if let Some(name) = undefined_symbol(message) {
                    for &index in &remaining {
                        if lines[index].contains(name) {
                            verdict.rejected.insert(index);
                        }
                    }
                } else {
                    return Err(format!("{cpu}: unplaced vasm error: {message}"));
                }
            }
        }
        if !found_errors {
            return Ok(verdict);
        }
    }
}

/// The name in vasm's "undefined symbol <name>" message, which has no line.
fn undefined_symbol(message: &str) -> Option<&str> {
    let (_, rest) = message.split_once("undefined symbol <")?;
    rest.split('>').next()
}

fn assemble_source(source: &str, machine: &Machine) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::new("t.s");
    options.cpu = machine.cpu;
    options.fpu = machine.fpu;
    options.mmu_68851 = machine.mmu_68851;
    options.fixed_addresses = true;
    assemble(&options, &files)
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.iter().zip(b).position(|(x, y)| x != y);
    common.or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

/// Groups lines by mnemonic and suffix, with a count and some examples of each.
fn summarise(lines: &[String]) -> String {
    let mut groups: Vec<(&str, Vec<&str>)> = Vec::new();
    for line in lines {
        let head = line.split_whitespace().next().unwrap_or("");
        match groups.iter_mut().find(|group| group.0 == head) {
            Some(group) => group.1.push(line),
            None => groups.push((head, vec![line])),
        }
    }
    let mut out = String::new();
    for (head, members) in groups {
        let step = (members.len() / 12).max(1);
        let examples: Vec<&str> = members
            .iter()
            .step_by(step)
            .take(12)
            .map(|line| line.trim_start().strip_prefix(head).unwrap_or(line).trim())
            .collect();
        out.push_str(&format!(
            "  {head} x{}: {}\n",
            members.len(),
            examples.join("  ")
        ));
    }
    out
}

/// What one CPU's sweep found, as text; empty if it matched.
fn sweep_machine(vasm: &Vasm, machine: &Machine) -> Result<String, String> {
    let name = format!("{} {:?}", machine.name, machine.vasm_args);
    let mut wrongly_accepted = Vec::new();
    let mut wrongly_rejected = Vec::new();
    let mut accepted = Vec::new();
    let mut total = 0;
    for mnemonic in MNEMONICS.iter().chain(FPU_MNEMONICS).chain(MMU_MNEMONICS) {
        let lines = lines_for(mnemonic);
        total += lines.len();
        let verdict = vasm_verdict(vasm, &lines, machine)?;
        for (index, line) in lines.into_iter().enumerate() {
            let we_accept = !assemble_source(&format!("{line}\n"), machine).has_errors();
            match (we_accept, verdict.rejected.contains(&index)) {
                (true, true) if vasm_wrongly_rejects(&line) => {}
                (true, true) => wrongly_accepted.push(line),
                (false, false) if verdict.lenient.contains(&index) => {}
                (false, false) if stricter_than_vasm(&line) => {}
                (false, false) => wrongly_rejected.push(line),
                (true, false) => accepted.push(line),
                (false, true) => {}
            }
        }
    }

    let mut report = String::new();
    if !wrongly_accepted.is_empty() || !wrongly_rejected.is_empty() {
        report = format!(
            "{name}, {total} lines:\nwe accept, vasm rejects ({}):\n{}we reject, vasm accepts ({}):\n{}",
            wrongly_accepted.len(),
            summarise(&wrongly_accepted),
            wrongly_rejected.len(),
            summarise(&wrongly_rejected),
        );
        return Ok(report);
    }

    // Assemble every line accepted by both, together, and use our line table
    // to name the first difference.
    let source = accepted
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<Vec<_>>()
        .concat();
    let theirs = vasm
        .assemble_with_args(&source, machine.name, Format::Bin, machine.vasm_args)
        .map_err(|error| error.to_string())?;
    let assembly = assemble_source(&source, machine);
    let empty = Vec::new();
    let bytes = assembly.sections.first().map_or(&empty, |s| &s.data);
    if let Some(offset) = first_difference(bytes, &theirs.bytes) {
        let line = assembly
            .line_for_address(0, offset as u32)
            .and_then(|entry| accepted.get(entry.span.line as usize - 1));
        report.push_str(&format!(
            "{name}: {line:?} at offset {offset:#x}: ours {:02x?} vasm {:02x?}\n",
            bytes.get(offset..offset + 10),
            theirs.bytes.get(offset..offset + 10),
        ));
    }
    Ok(report)
}

#[test]
fn test_assemble_sweep_matches_vasm_on_every_cpu() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let reports: Vec<Result<String, String>> = std::thread::scope(|scope| {
        let handles: Vec<_> = MACHINES
            .iter()
            .map(|machine| {
                let vasm = &vasm;
                scope.spawn(move || sweep_machine(vasm, machine))
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| {
                handle
                    .join()
                    .unwrap_or_else(|_| Err("a sweep thread panicked".to_string()))
            })
            .collect()
    });
    let mut failures = String::new();
    for report in reports {
        failures.push_str(&report?);
    }
    assert!(failures.is_empty(), "{failures}");
    Ok(())
}
