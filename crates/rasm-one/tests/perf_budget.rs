//! Holds assembly to the brief's "from memory in milliseconds". The budget only
//! means something for an optimised build, so CI runs this with `--release`.

use std::error::Error;
use std::fmt::Write;
use std::time::{Duration, Instant};

use rasm_one::{assemble, write_hunk_exe, MemoryProvider, Options};

/// Lines of generated source.
const LINES: usize = 50_000;

/// Far above what is measured, so a busy CI machine doesn't fail it, but close
/// enough that an accidental quadratic step will.
const BUDGET: Duration = Duration::from_millis(250);

/// A source of typical shape: labels, instructions, data, macro calls.
fn generated_source() -> Result<String, std::fmt::Error> {
    let mut source = String::from(
        "add3 macro\n move.\\0 \\1,d0\n add.\\0 #3,d0\n move.\\0 d0,\\2\n endm\n\
         base equ $dff000\n section code,code\nstart:\n",
    );
    let mut lines = 0;
    let mut block = 0;
    while lines < LINES {
        writeln!(source, "label{block}:\n.loop:\tmove.l\t(a0)+,d0 ; copy")?;
        writeln!(source, "\tlea\tbase+{block}&$ff,a1\n\tmove.w\td0,$180(a1)")?;
        writeln!(source, "\tadd3.w\t4(a2,d1.w),(a3)")?;
        writeln!(source, "\tdbra\td7,.loop\n\tbsr\tlabel{block}\n\trts")?;
        writeln!(source, "\tdc.w\t1,2,3,.loop-label{block}\n\tcnop\t0,4")?;
        lines += 10;
        block += 1;
    }
    Ok(source)
}

#[test]
fn test_assemble_large_source_within_budget() -> Result<(), Box<dyn Error>> {
    if cfg!(debug_assertions) {
        eprintln!("skipping: the budget applies to optimised builds");
        return Ok(());
    }
    let source = generated_source()?;
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let options = Options::new("t.s");

    // The fastest of several runs is the least disturbed by other work.
    let mut best = Duration::MAX;
    for _ in 0..5 {
        let start = Instant::now();
        let assembly = assemble(&options, &files);
        assert!(!assembly.has_errors(), "{:?}", assembly.diagnostics.first());
        let bytes = write_hunk_exe(&assembly)?;
        best = best.min(start.elapsed());
        assert!(!bytes.is_empty());
    }
    eprintln!("{LINES} lines in {best:?}");
    assert!(
        best <= BUDGET,
        "{LINES} lines took {best:?}, budget {BUDGET:?}"
    );
    Ok(())
}
