//! Sweeps every mnemonic, size and operand class against vasm -no-opt.
//!
//! The mnemonic list is independent of the table, and the sweep covers
//! combinations the table calls invalid, so a missing form can't go unnoticed.

use std::collections::HashSet;
use std::error::Error;

use rasm_one::{assemble, Assembly, MemoryProvider, Options};
use rasm_oracle::{Format, Vasm};

const MNEMONICS: &[&str] = &[
    "abcd", "add", "adda", "addi", "addq", "addx", "and", "andi", "asl", "asr", "bcc", "bchg",
    "bclr", "bcs", "beq", "bge", "bgt", "bhi", "bhs", "ble", "blo", "bls", "blt", "bmi", "bne",
    "bpl", "bra", "bset", "bsr", "btst", "bvc", "bvs", "chk", "clr", "cmp", "cmpa", "cmpi", "cmpm",
    "dbcc", "dbcs", "dbeq", "dbf", "dbge", "dbgt", "dbhi", "dbhs", "dble", "dblo", "dbls", "dblt",
    "dbmi", "dbne", "dbpl", "dbra", "dbt", "dbvc", "dbvs", "divs", "divu", "eor", "eori", "exg",
    "ext", "illegal", "jmp", "jsr", "lea", "link", "lsl", "lsr", "move", "movea", "movem", "movep",
    "moveq", "muls", "mulu", "nbcd", "neg", "negx", "nop", "not", "or", "ori", "pea", "reset",
    "rol", "ror", "roxl", "roxr", "rte", "rtr", "rts", "sbcd", "scc", "scs", "seq", "sf", "sge",
    "sgt", "shi", "shs", "sle", "slo", "sls", "slt", "smi", "sne", "spl", "st", "stop", "sub",
    "suba", "subi", "subq", "subx", "svc", "svs", "swap", "tas", "trap", "trapv", "tst", "unlk",
];

const SIZES: &[&str] = &["", ".b", ".w", ".l", ".s"];

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

/// Reviewed cases where we deliberately reject what vasm accepts.
fn stricter_than_vasm(line: &str) -> bool {
    // vasm treats `movea` to anything but an address register as `move`. The
    // PRM defines `movea` only for address registers.
    let movea_to_non_address = line.starts_with(" movea") && !line.ends_with(",a5");
    // vasm allows an address as a `movem` mask in raw binaries, where addresses
    // are constants, but not in hunk output. We require a constant everywhere.
    let address_as_mask =
        line.starts_with(" movem") && (line.contains(" *+8,") || line.ends_with(",*+8"));
    movea_to_non_address || address_as_mask
}

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

fn lines() -> Vec<String> {
    let mut lines = Vec::new();
    for mnemonic in MNEMONICS {
        for size in SIZES {
            lines.push(format!(" {mnemonic}{size}"));
            for a in OPERANDS {
                lines.push(format!(" {mnemonic}{size} {a}"));
                for b in SECOND_OPERANDS {
                    lines.push(format!(" {mnemonic}{size} {a},{b}"));
                }
            }
        }
    }
    lines
}

/// Asks vasm which lines it rejects, resuming after fatal errors, which stop
/// it on the spot.
fn vasm_rejected(vasm: &Vasm, lines: &[String]) -> Result<HashSet<usize>, Box<dyn Error>> {
    let mut rejected = HashSet::new();
    let mut start = 0;
    while start < lines.len() {
        let source = lines[start..].join("\n") + "\n";
        let out = vasm.assemble(&source, "68000", Format::Bin)?;
        let mut fatal = None;
        for line in out.stderr.lines() {
            let is_fatal = line.starts_with("fatal error");
            if !is_fatal && !line.starts_with("error") {
                continue;
            }
            let Some(number) = line
                .split(" in line ")
                .nth(1)
                .and_then(|rest| rest.split_whitespace().next())
                .and_then(|n| n.parse::<usize>().ok())
            else {
                continue;
            };
            rejected.insert(start + number - 1);
            if is_fatal {
                fatal = Some(start + number - 1);
            }
        }
        match fatal {
            Some(line) => start = line + 1,
            None => break,
        }
    }
    Ok(rejected)
}

fn assemble_source(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    assemble(&Options::new("t.s"), &files)
}

/// True if we accept `line` on its own.
fn we_accept(line: &str) -> bool {
    !assemble_source(&format!("{line}\n")).has_errors()
}

fn first_difference(a: &[u8], b: &[u8]) -> Option<usize> {
    let common = a.iter().zip(b).position(|(x, y)| x != y);
    common.or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())))
}

#[test]
fn test_assemble_sweep_matches_vasm_availability_and_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let lines = lines();
    let rejected = vasm_rejected(&vasm, &lines)?;

    let mut wrongly_accepted = Vec::new();
    let mut wrongly_rejected = Vec::new();
    let mut accepted = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        match (we_accept(line), rejected.contains(&index)) {
            (true, true) => wrongly_accepted.push(line.as_str()),
            (false, false) if stricter_than_vasm(line) => {}
            (false, false) => wrongly_rejected.push(line.as_str()),
            (true, false) => accepted.push(line.as_str()),
            (false, true) => {}
        }
    }

    // Assemble every line accepted by both, together, and use our line table
    // to name the first difference.
    let source = accepted
        .iter()
        .map(|line| format!("{line}\n"))
        .collect::<Vec<_>>()
        .concat();
    let mut mismatch = None;
    if wrongly_accepted.is_empty() && wrongly_rejected.is_empty() {
        let theirs = vasm.assemble(&source, "68000", Format::Bin)?;
        let assembly = assemble_source(&source);
        let bytes = &assembly.sections[0].data;
        if let Some(offset) = first_difference(bytes, &theirs.bytes) {
            let line = assembly
                .line_for_address(0, offset as u32)
                .and_then(|entry| accepted.get(entry.span.line as usize - 1));
            mismatch = Some(format!(
                "{line:?} at offset {offset:#x}: ours {:02x?} vasm {:02x?}",
                bytes.get(offset..offset + 8),
                theirs.bytes.get(offset..offset + 8),
            ));
        }
    }

    let sample = |list: &[&str]| {
        list.iter()
            .take(400)
            .copied()
            .collect::<Vec<_>>()
            .join("\n")
    };
    assert!(
        wrongly_accepted.is_empty() && wrongly_rejected.is_empty() && mismatch.is_none(),
        "of {} lines ({} accepted by vasm):\n\
         we accept, vasm rejects ({}):\n{}\n\
         we reject, vasm accepts ({}):\n{}\n\
         byte mismatch: {mismatch:?}",
        lines.len(),
        lines.len() - rejected.len(),
        wrongly_accepted.len(),
        sample(&wrongly_accepted),
        wrongly_rejected.len(),
        sample(&wrongly_rejected),
    );
    Ok(())
}
