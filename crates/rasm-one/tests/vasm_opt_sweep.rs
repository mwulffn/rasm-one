//! Tries every combination of mnemonic, size, value and operand with the optimiser on, one
//! line at a time, and compares each with vasm. Hand-picked examples missed real bugs here.
//!
//! Set `RASM_VASM_CACHE` to keep vasm's answers, as this runs it for every line.

use std::error::Error;
use std::sync::Mutex;

use rasm_one::{
    assemble, write_bin, write_hunk_object, Cpu, MemoryProvider, Optimisations, Options, Rule,
    Target,
};
use rasm_oracle::{Format, Vasm};

const CPUS: &[&str] = &["68000", "68010", "68020", "68060"];
const SIZES: &[&str] = &["", ".b", ".w", ".l"];
const VALUES: &[&str] = &[
    "0",
    "z",
    "1",
    "8",
    "9",
    "-1",
    "-8",
    "-9",
    "127",
    "128",
    "254",
    "255",
    "$7fff",
    "$8000",
    "$ffff",
    "$ffff8000",
    "$ffff7fff",
    "$10000",
    "$7f0000",
    "$ffffffff",
    "f",
];
const DESTINATIONS: &[&str] = &[
    "d3", "a3", "sp", "(a3)", "(a3)+", "-(a3)", "4(a3)", "0(a3)", "4", "4.l", "$10000", "f",
];
const LISTS: &[&str] = &[
    "d0", "a1", "d0-d1", "a1/a2", "a0/a1", "#0", "#1", "#$200", "#$600", "$200", "0",
];
const MEMORY: &[&str] = &[
    "(a0)", "(a1)", "(a0)+", "(a1)+", "-(a0)", "4(a1)", "0(a1)", "0", "0.l", "$100", "$100.l",
    "0.w", "$200", "f", "f(pc)",
];
const ADDRESSES: &[&str] = &[
    "0",
    "0.w",
    "0.l",
    "4",
    "4.l",
    "$8000",
    "f",
    "f.l",
    "(a0)",
    "(a1)",
    "0(a0)",
    "0(a1)",
    "4(a0)",
    "-8(a0)",
    "9(a0)",
    "4(a1)",
    "32767(a0)",
    "32768(a1)",
    "100000(a0)",
    "100000(a1)",
    "(f,pc)",
    "(0,a0,d1.w)",
];

fn lines() -> Vec<String> {
    let mut lines = Vec::new();
    for mnemonic in [
        "add", "adda", "addi", "sub", "suba", "subi", "cmp", "cmpa", "cmpi", "and", "andi", "or",
        "ori", "eor", "eori", "move", "movea",
    ] {
        for size in SIZES {
            for value in VALUES {
                for destination in DESTINATIONS {
                    lines.push(format!(" {mnemonic}{size} #{value},{destination}"));
                }
            }
        }
    }
    for size in ["", ".w", ".l"] {
        for list in LISTS {
            for memory in MEMORY {
                lines.push(format!(" movem{size} {list},{memory}"));
                lines.push(format!(" movem{size} {memory},{list}"));
            }
        }
    }
    for address in ADDRESSES {
        for register in ["a0", "a1", "sp"] {
            lines.push(format!(" lea {address},{register}"));
        }
        lines.push(format!(" jmp {address}"));
        lines.push(format!(" jsr {address}"));
        lines.push(format!(" pea {address}"));
    }
    for base in ["0", "4", "z", "f", "0.w", "0.l", "$8000"] {
        for form in [
            format!("([{base},pc])"),
            format!("([{base},pc],4)"),
            format!("([{base},pc,d1.w])"),
            format!("([{base},a0])"),
            format!("([{base},a0],0)"),
            format!("({base},a0,d1.w)"),
            format!("({base},pc,d1.w)"),
            format!("({base},d1.w)"),
        ] {
            lines.push(format!(" move.w {form},d0"));
        }
    }
    for mnemonic in ["clr", "tst", "not", "neg"] {
        for size in SIZES {
            for destination in DESTINATIONS {
                lines.push(format!(" {mnemonic}{size} {destination}"));
            }
        }
    }
    for mnemonic in ["asl", "asr", "lsl", "lsr", "rol"] {
        for size in SIZES {
            for count in ["#1", "#2", "#8", "d1"] {
                lines.push(format!(" {mnemonic}{size} {count},d3"));
            }
            lines.push(format!(" {mnemonic}{size} d3"));
            lines.push(format!(" {mnemonic}{size} (a3)"));
        }
    }
    for size in ["", ".w", ".l"] {
        for value in [
            "0", "z", "-4", "32767", "-32768", "-32769", "32768", "$10000",
        ] {
            lines.push(format!(" link{size} a6,#{value}"));
        }
    }
    lines
}

/// What vasm gets wrong, or rasm-one refuses by choice.
fn known_difference(line: &str, cpu: &str) -> bool {
    let words: Vec<&str> = line.split_whitespace().collect();
    let (Some(mnemonic), Some(operands)) = (words.first(), words.get(1)) else {
        return false;
    };
    let base = mnemonic.split('.').next().unwrap_or(mnemonic);
    let size = mnemonic.split('.').nth(1);
    let single_register = operands.len() == 2 && operands.starts_with('d');
    // vasm bug 17: `asl d1` becomes `add d1,d0`.
    if base == "asl" && single_register && operands != &"d0" && cpu != "68060" {
        return true;
    }
    // vasm bug 15: `eor.b #$ffff,d0` isn't an error with `og`.
    let low_byte_set = ["#$ffff,", "#$7fff,", "#$ffff7fff,"];
    let out_of_range = low_byte_set.iter().any(|value| operands.starts_with(value));
    if matches!(base, "eor" | "eori") && size == Some("b") && out_of_range {
        return true;
    }
    // rasm-one refuses `movea` to anything but an address register, by choice.
    if base == "movea" && !to_address(operands) {
        return true;
    }
    // Two bare addresses, either of which could be the list. Writing no registers does nothing
    // either way, so it doesn't matter that vasm keeps the instruction.
    if base == "movem" {
        let bare = |text: &str| {
            text.chars()
                .next()
                .is_some_and(|c| c == '$' || c.is_ascii_digit())
        };
        let zero_list = operands.starts_with("0,") || operands.starts_with("0.");
        if operands.split(',').all(bare) && zero_list {
            return true;
        }
    }
    // vasm bug 14: subtracting $8000 from an address register.
    if matches!(base, "sub" | "suba") && operands.starts_with("#$8000,") && to_address(operands) {
        return true;
    }
    false
}

fn to_address(operands: &str) -> bool {
    operands.ends_with(",a3") || operands.ends_with(",sp")
}

fn ours(line: &str, cpu: &str) -> Result<Vec<u8>, rasm_one::OutputError> {
    let mut files = MemoryProvider::new();
    files.insert("t.s", format!("{line}\nf: nop\nz equ 0\n"));
    let mut options = Options::new("t.s");
    options.cpu = cpu
        .parse()
        .ok()
        .and_then(Cpu::from_number)
        .unwrap_or_default();
    write_hunk_object(&assemble(&options, &files))
}

fn hex(bytes: &[u8]) -> String {
    let start = bytes
        .windows(4)
        .position(|window| window == [0, 0, 3, 0xE9])
        .map_or(0, |at| at + 8);
    let end = (start + 16).min(bytes.len());
    bytes[start..end]
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[test]
fn test_assemble_every_combination_matches_optimising_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let lines = lines();
    let found = Mutex::new(Vec::new());
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        for chunk in lines.chunks(lines.len() / threads + 1) {
            let (vasm, found) = (&vasm, &found);
            scope.spawn(move || {
                for line in chunk {
                    for cpu in CPUS {
                        if known_difference(line, cpu) {
                            continue;
                        }
                        let source = format!("{line}\nf: nop\nz equ 0\n");
                        let theirs = match vasm.assemble_cached(
                            &source,
                            cpu,
                            Format::HunkObject,
                            &["-opt-o+"],
                        ) {
                            Ok(theirs) => theirs,
                            Err(error) => {
                                if let Ok(mut found) = found.lock() {
                                    found.push(format!("{cpu} {line}: vasm failed: {error}"));
                                }
                                continue;
                            }
                        };
                        let mine = ours(line, cpu);
                        let same = match &mine {
                            Ok(bytes) => theirs.success && *bytes == theirs.bytes,
                            Err(_) => !theirs.success,
                        };
                        if !same {
                            let text = match &mine {
                                Ok(bytes) => hex(bytes),
                                Err(_) => "refused".to_string(),
                            };
                            let answer = if theirs.success {
                                hex(&theirs.bytes)
                            } else {
                                "refused".to_string()
                            };
                            if let Ok(mut found) = found.lock() {
                                found.push(format!("{cpu} {line}: vasm {answer} ours {text}"));
                            }
                        }
                    }
                }
            });
        }
    });
    let mut found = found.into_inner().map_err(|_| "a thread panicked")?;
    found.sort();
    assert!(
        found.is_empty(),
        "{} differences in {} lines:\n{}",
        found.len(),
        lines.len(),
        found.join("\n")
    );
    Ok(())
}

/// What the hunk test leaves out: final addresses, where labels and numbers are the same thing.
#[test]
fn test_assemble_org_combinations_match_optimising_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut lines = Vec::new();
    for org in ["$100", "$8000", "$20000", "$80000000"] {
        for address in [
            "f",
            "f.l",
            "4",
            "$100",
            "$7ffe",
            "$8000",
            "$20008",
            "$80000008",
            "(f,pc)",
        ] {
            for form in [
                format!(" lea {address},a0"),
                format!(" jmp {address}"),
                format!(" jsr {address}"),
                format!(" pea {address}"),
                format!(" move.l {address},d0"),
                format!(" move.l d0,{address}"),
                format!(" tst.w {address}"),
                format!(" btst #1,{address}"),
                format!(" cmpi.l #1,{address}"),
                format!(" move.l #{address},a0"),
                format!(" bra {address}"),
                format!(" bsr {address}"),
                format!(" beq {address}"),
                format!(" bra.w {address}"),
            ] {
                // Only an instruction that takes an address can have one with a size or `pc`,
                // and rasm-one is stricter about that than vasm.
                let plain = !address.contains('.') && !address.contains('(');
                let takes_address = !form.contains('#') && !form.contains(" b");
                if !plain && !takes_address {
                    continue;
                }
                lines.push(format!(" org {org}\n{form}\n nop\nf: nop"));
            }
        }
    }
    let mut found = Vec::new();
    for source in &lines {
        for cpu in ["68000", "68020"] {
            for flags in [
                &["-opt-o+"][..],
                &["-opt-o-", "-opt-a+"],
                &["-opt-o+", "-opt-o3-"],
            ] {
                let theirs = vasm.assemble_cached(source, cpu, Format::Bin, flags)?;
                let mut files = MemoryProvider::new();
                files.insert("t.s", source.as_str());
                let mut options = Options::new("t.s");
                options.target = Target::RawBinary;
                options.cpu = cpu
                    .parse()
                    .ok()
                    .and_then(Cpu::from_number)
                    .unwrap_or_default();
                if flags[0] == "-opt-o-" {
                    options.optimisations = Optimisations::none();
                    options.optimisations.set_rule(Rule::PcRelative, true);
                } else if flags.len() > 1 {
                    options.optimisations.set_rule(Rule::Absolutes, false);
                }
                let mine = write_bin(&assemble(&options, &files));
                let same = match &mine {
                    Ok(bytes) => theirs.success && *bytes == theirs.bytes,
                    Err(_) => !theirs.success,
                };
                if !same {
                    found.push(format!(
                        "{cpu} {flags:?} {source:?}: vasm {} ours {}",
                        if theirs.success {
                            hex(&theirs.bytes)
                        } else {
                            "refused".to_string()
                        },
                        mine.map_or("refused".to_string(), |bytes| hex(&bytes)),
                    ));
                }
            }
        }
    }
    assert!(
        found.is_empty(),
        "{} differences:\n{}",
        found.len(),
        found.join("\n")
    );
    Ok(())
}
