//! Runs assembled executables under vamos, which emulates a 68k CPU and
//! AmigaDOS's loader. The program's exit code shows that it really worked.
//! `RASM_VAMOS` gives the path; without it this skips locally and fails in CI.

use std::error::Error;
use std::fs;
use std::process::Command;

use rasm_one::{assemble, write_hunk_exe_with, HunkExeOptions, MemoryProvider, Options};

/// Follows a pointer across hunks and sums values from a macro, a relocated
/// table and BSS, so wrong relocations, sections or encodings change the result.
const PROGRAM: &str = r#"
add_to macro
    add.\0 \1,d0
    endm

    section code,code
start:
    moveq   #0,d0
    move.l  table,a0            ; relocated to data
    move.l  (a0)+,d1
    add_to.l d1
    lea     space,a1            ; relocated to BSS
    move.l  #30,(a1)
    add_to.l (a1)
    bsr.s   helper
    add_to.l #2
    rts
helper:
    addq.l  #4,d0
.done:
    rts

    section numbers,data
table:
    dc.l    values
values:
    dc.l    6

    section scratch,bss
space:
    ds.l    1
"#;

/// 6 + 30 + 4 + 2.
const EXPECTED_EXIT_CODE: i32 = 42;

#[test]
fn test_vamos_runs_executables_in_both_relocation_formats() -> Result<(), Box<dyn Error>> {
    let Some(vamos) = std::env::var_os("RASM_VAMOS") else {
        assert!(
            std::env::var_os("CI").is_none(),
            "RASM_VAMOS is not set but CI requires it"
        );
        eprintln!("skipping: RASM_VAMOS not set");
        return Ok(());
    };
    let mut files = MemoryProvider::new();
    files.insert("t.s", PROGRAM);
    let assembly = assemble(&Options::new("t.s"), &files);
    assert!(!assembly.has_errors(), "{:?}", assembly.diagnostics);

    let dir = std::env::temp_dir().join(format!("rasm-vamos-{}", std::process::id()));
    fs::create_dir_all(&dir)?;
    for kickstart1 in [false, true] {
        let bytes = write_hunk_exe_with(&assembly, HunkExeOptions { kickstart1 })?;
        fs::write(dir.join("prog"), bytes)?;
        let output = Command::new(&vamos)
            .current_dir(&dir)
            .arg("prog")
            .output()?;
        assert_eq!(
            output.status.code(),
            Some(EXPECTED_EXIT_CODE),
            "kickstart1={kickstart1}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    // Best effort: a leftover temp dir must not fail the test.
    let _ = fs::remove_dir_all(&dir);
    Ok(())
}
