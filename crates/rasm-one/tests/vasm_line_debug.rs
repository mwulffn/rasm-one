//! Compares `HUNK_DEBUG` line information with vasm's `-linedebug`, byte for byte.

use std::error::Error;

use rasm_one::{
    assemble, link, write_hunk_exe, write_hunk_object, Assembly, MemoryProvider, Options,
};
use rasm_oracle::{Format, Vasm};

/// Files beside the source, for includes.
const FILES: &[(&str, &[u8])] = &[
    ("e.s", b" nop\n"),
    ("sub/inc.i", b"v equ 1\n nop\n rts\n"),
    ("data.bin", b"abc"),
];

const SOURCES: &[&str] = &[
    "",
    " nop",
    " nop\n\n nop\n ; comment\n nop",
    "x equ 1",
    "start: nop\nnext: rts",
    " dc.b 1\n even\n dc.w 1",
    " nop\n dc.b 1\n even\n dc.w 1\n\n ds.b 4\n cnop 0,4\n nop\n bra f\n dc.l f\nf: rts",
    " dc.b 1,2,3\n dc.b 4\n nop",
    " nop\n ds.l 2\n dcb.w 2,1\n nop",
    " nop\n ds.w 0\n nop",
    " nop\n cnop 0,4\n nop\n cnop 0,8\n dc.b 1",
    " dc.b 1\n nop\n nop",
    " include \"e.s\"\n nop",
    " nop\n include \"e.s\"\n nop\n include \"sub/inc.i\"\n nop",
    " include \"e.s\"\n include \"e.s\"",
    " rept 2\n include \"e.s\"\n endr\n nop",
    " nop\n incbin \"data.bin\"\n even\n nop",
    " rept 3\n nop\n nop\n endr\n nop",
    " rept 2\n nop\n endr",
    "m macro\n nop\n endm\n m\n m\n nop\n m",
    "m macro\n nop\n dc.w \\1\n endm\n m 1\n m 2",
    " section c,code\n nop\n section d,data\n dc.w 1\n dc.w 2\n section b,bss\n ds.w 1\n section e,code\n nop",
    " nop\n section a,code\n nop\n section CODE,code\n nop\n section a,code\n nop",
    " section b,bss\n ds.w 0",
    " section b,bss\n ds.w 1\n ds.l 2",
    " opt d-\n nop",
    " nop\n opt d-",
    " opt d+\n nop",
    "a: nop\n opt x+\n nop\n xdef a",
    " nop\n nop\n xref q\n jmp q",
    " move.l #2,d0\n asl #1,d1\n lea 4(a0),a1",
    " bra f\nf:\n nop",
    " if 1\n nop\n else\n rts\n endc\n nop",
];

fn ours(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    for (path, contents) in FILES {
        files.insert(*path, contents.to_vec());
    }
    let mut options = Options::new("t.s");
    options.line_debug = true;
    assemble(&options, &files)
}

fn differences(format: Format) -> Result<Vec<String>, Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(Vec::new());
    };
    let mut failures = Vec::new();
    for source in SOURCES {
        let theirs = vasm.assemble_with_files_and_args(
            source,
            "68000",
            format,
            FILES,
            &["-linedebug", "-noabspath", "-opt-o+"],
        )?;
        let assembly = ours(source);
        let mine = match format {
            Format::HunkObject => write_hunk_object(&assembly),
            _ => write_hunk_exe(&assembly),
        };
        let same = match &mine {
            Ok(bytes) => theirs.success && *bytes == theirs.bytes,
            Err(_) => !theirs.success,
        };
        if !same {
            failures.push(format!(
                "{source:?}\n  vasm {:02x?} {}\n  ours {mine:02x?}",
                theirs.bytes, theirs.stderr
            ));
        }
    }
    Ok(failures)
}

#[test]
fn test_write_hunk_object_line_debug_matches_vasm() -> Result<(), Box<dyn Error>> {
    let failures = differences(Format::HunkObject)?;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_exe_line_debug_matches_vasm() -> Result<(), Box<dyn Error>> {
    let failures = differences(Format::HunkExe)?;
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_write_hunk_exe_has_no_line_debug_unless_asked() -> Result<(), Box<dyn Error>> {
    let mut files = MemoryProvider::new();
    files.insert("t.s", " nop\n");
    let bytes = write_hunk_exe(&assemble(&Options::new("t.s"), &files))?;
    assert!(!bytes.windows(4).any(|window| window == b"LINE"));
    Ok(())
}

#[test]
fn test_link_keeps_line_debug_at_joined_offsets() -> Result<(), Box<dyn Error>> {
    let inputs = [ours(" nop\n nop"), ours(" rts")];
    let bytes = write_hunk_exe(&link(&inputs))?;
    let tables = bytes.windows(4).filter(|window| *window == b"LINE").count();
    assert_eq!(tables, 2);
    Ok(())
}
