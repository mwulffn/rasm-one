//! Links with rasm-one and vlink, then compares what a loader would put in memory.
//!
//! vlink makes other choices in the file, such as its relocation hunks, so the bytes of the
//! files differ. Without `RASM_VLINK`, the path of a vlink binary, the comparisons skip.

use std::error::Error;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

use rasm_one::{
    assemble, link, write_hunk_exe, write_hunk_object, Assembly, MemoryProvider, Options,
};

fn assembled(source: &str, name: &str, options: &Options) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert(name, source);
    let mut options = options.clone();
    options.main = name.to_string();
    assemble(&options, &files)
}

fn word(bytes: &[u8], at: usize) -> Option<u32> {
    let field = bytes.get(at..at + 4)?;
    Some(u32::from_be_bytes(field.try_into().ok()?))
}

/// A hunk's memory flags and contents, with relocations applied as if hunk `n` were at
/// `(n + 1) << 24`.
fn loaded(file: &[u8]) -> Option<Vec<(u32, Vec<u8>)>> {
    let base = |hunk: u32| (hunk + 1) << 24;
    let mut at = 0;
    if word(file, at)? != 0x3F3 {
        return None;
    }
    at += 4;
    while word(file, at)? != 0 {
        at += 4 + word(file, at)? as usize * 4;
    }
    at += 4;
    let count = word(file, at)? as usize;
    at += 12;
    let mut hunks = Vec::new();
    for _ in 0..count {
        let size = word(file, at)?;
        hunks.push((size >> 30, vec![0u8; (size & 0x3FFF_FFFF) as usize * 4]));
        at += 4;
    }
    let mut current = 0;
    while at < file.len() {
        let kind = word(file, at)? & 0x3FFF_FFFF;
        at += 4;
        match kind {
            0x3E9 | 0x3EA => {
                let length = word(file, at)? as usize * 4;
                at += 4;
                let data = file.get(at..at + length)?;
                hunks.get_mut(current)?.1[..length].copy_from_slice(data);
                at += length;
            }
            0x3EB => at += 4,
            0x3EC | 0x3F7 | 0x3FC => {
                let short = kind != 0x3EC;
                let read = |at: &mut usize| -> Option<u32> {
                    if short {
                        let value = u16::from_be_bytes(file.get(*at..*at + 2)?.try_into().ok()?);
                        *at += 2;
                        Some(u32::from(value))
                    } else {
                        let value = word(file, *at)?;
                        *at += 4;
                        Some(value)
                    }
                };
                loop {
                    let offsets = read(&mut at)?;
                    if offsets == 0 {
                        break;
                    }
                    let target = read(&mut at)?;
                    for _ in 0..offsets {
                        let offset = read(&mut at)? as usize;
                        let contents = &mut hunks.get_mut(current)?.1;
                        let value = word(contents, offset)?.wrapping_add(base(target));
                        contents[offset..offset + 4].copy_from_slice(&value.to_be_bytes());
                    }
                }
                if short && at % 4 != 0 {
                    at += 2;
                }
            }
            0x3F0 => loop {
                let length = word(file, at)?;
                at += 4;
                if length == 0 {
                    break;
                }
                at += (length & 0xFF_FFFF) as usize * 4 + 4;
            },
            0x3F2 => current += 1,
            _ => return None,
        }
    }
    Some(hunks)
}

struct Vlinked {
    success: bool,
    bytes: Vec<u8>,
    stderr: String,
}

fn vlink(tool: &str, objects: &[Vec<u8>], name: &str) -> Result<Vlinked, Box<dyn Error>> {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("rasm-vlink-{}-{name}", std::process::id()));
    fs::create_dir_all(&dir)?;
    let mut paths = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        let path = dir.join(format!("{index}.o"));
        fs::write(&path, object)?;
        paths.push(path);
    }
    let out = dir.join("out");
    let run = Command::new(tool)
        .args(["-bamigahunk", "-o"])
        .arg(&out)
        .args(&paths)
        .output()?;
    let bytes = fs::read(&out).unwrap_or_default();
    // Best effort: a leftover temp dir must not fail the test.
    let _ = fs::remove_dir_all(&dir);
    Ok(Vlinked {
        success: run.status.success(),
        bytes,
        stderr: String::from_utf8_lossy(&run.stderr).into_owned(),
    })
}

const PROGRAMS: &[&[&str]] = &[
    &[" xdef s\ns: jsr h\n xref h\n rts", " xdef h\nh: rts"],
    &[" xref h\n bra h\n nop", " nop\n nop\n xdef h\nh: rts"],
    &[" xref h\n bsr.s h\n", " dc.b 1\n even\n xdef h\nh: rts"],
    &[
        " nop\n xdef a\na: rts\n",
        " xref a\n bsr a\n bra a\n lea a(pc),a0",
    ],
    &[
        " xref v\n move.l #v,d0\n move.w #v,d1\n move.b #v,d2\n dc.l v",
        " xdef v\nv equ 5",
    ],
    &[" xref v\n jmp v\n move.w v.w,d0", " xdef v\nv equ $100"],
    &[
        " section c,code\n dc.b 1,2,3\n section c,code\n dc.l x\n xref x",
        " section c,code\n xdef x\nx: dc.b 9",
    ],
    &[
        " section d,data\n dc.b 1\n xdef p\np: dc.b 2",
        " xref p\n section d,data\n dc.l p+1",
    ],
    &[
        " section b,bss\n xdef q\nq: ds.b 3",
        " section b,bss\nz: ds.b 5\n section t,code\n xref q\n move.l q,d0\n move.l z,d1",
    ],
    &[
        " section d,data\n dc.w 1\n section t,code\n nop",
        " section t,code\n rts\n section d,data\n dc.l 2",
        " section e,data\n dc.l 3\n section t,code\n nop",
    ],
    &[
        " section c,code_c\n nop",
        " section c,code_f\n rts",
        " section c,code\n rts",
    ],
    &[
        " section a,code\n xref b\n jmp b\n dc.l b",
        " section z,code\n nop\n xdef b\nb: rts",
    ],
    &[
        " section a,code\n xref d\n lea d,a0\n move.l d+4,d0",
        " section x,data\n dc.l 0\n xdef d\nd: dc.l 1,2",
    ],
    &[
        " xref x\n dc.l x-4,x+4\n move.l x+8,d0",
        " nop\n xdef x\nx: rts",
    ],
    &[
        " section a,code\n nop\n xdef f\nf: nop\n dc.l f",
        " section a,code\n xref f\n dc.l f\n bra f",
    ],
    &[
        "s: nop\n xref t\n bra t\n ds.b 32000",
        " ds.b 32000\n xdef t\nt: rts",
    ],
    &[" nop", " nop", " nop\n rts"],
    &[
        " section c,code\n nop",
        " section c,code_c\n rts",
        " section c,code_f\n rts",
    ],
    &[
        " section c,code_f\n nop",
        " section c,code\n rts",
        " section c,code_c\n rts",
    ],
    &[" section d,data_c\n dc.w 1", " section d,data\n dc.w 2"],
    &[" section c,code\n nop", " section c,data\n dc.w 2"],
    &[" section c,code_c\n nop\n section d,bss_c\n ds.w 1\n section c,code_c\n rts"],
    &[
        " xref m\n dc.l m",
        " section x,code\nm0 macro\n nop\n endm\n m0\n xdef m\nm: rts",
    ],
    &[
        " section d,data\n dc.l 0\n xdef g\ng: dc.l 0",
        " xref g\n move.w g(a0),d0\n move.w g(a4),d1",
    ],
    &[
        " xref x\n move.w x(pc,d0.w),d1\n lea x(pc),a0",
        " xdef x\nx: rts",
    ],
    &[" xref x\n dc.b x\n dc.w x", " xdef x\nx equ 3"],
    &[" dc.b 1", " dc.b 2,3", " dc.b 4,5,6"],
    &[
        " section bb,bss\n ds.w 1",
        " section dd,data\n dc.w 1",
        " section cc,code\n nop",
    ],
];

/// Programs that both must refuse.
const REFUSED: &[&[&str]] = &[
    &[" xref h\n jsr h"],
    &[" nop\nf: move.w f(a0),d0", " nop"],
    &[
        " section d,data\nf: dc.w 0\n section c,code\n move.w f(a0),d0",
        " nop",
    ],
    &[" xdef a\na: nop", " xdef a\na: rts"],
    &[" xref t\n bsr.s t\n ds.b 200", " ds.b 200\n xdef t\nt: rts"],
    &[
        " xref t\n bsr t\n ds.b 32000",
        " ds.b 32000\n xdef t\nt: rts",
    ],
];

fn tool() -> Option<String> {
    let tool = std::env::var("RASM_VLINK").ok();
    if tool.is_none() {
        eprintln!("skipping: RASM_VLINK not set");
    }
    tool
}

fn inputs(program: &[&str], options: &Options) -> Vec<Assembly> {
    program
        .iter()
        .enumerate()
        .map(|(index, source)| assembled(source, &format!("f{index}.s"), options))
        .collect()
}

#[test]
fn test_link_matches_what_vlink_loads() -> Result<(), Box<dyn Error>> {
    let Some(tool) = tool() else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for (index, program) in PROGRAMS.iter().enumerate() {
        for options in [Options::unoptimised("t.s"), Options::new("t.s")] {
            let inputs = inputs(program, &options);
            let objects: Vec<Vec<u8>> = inputs
                .iter()
                .map(write_hunk_object)
                .collect::<Result<_, _>>()?;
            let theirs = vlink(&tool, &objects, &index.to_string())?;
            let linked = link(&inputs);
            let ours = write_hunk_exe(&linked);
            let same = match &ours {
                Ok(bytes) => theirs.success && loaded(bytes) == loaded(&theirs.bytes),
                Err(_) => !theirs.success,
            };
            if !same || (loaded(&theirs.bytes).is_none() && theirs.success) {
                failures.push(format!(
                    "{program:?}\n  vlink {:02x?} {}\n  ours {:02x?} {:?}",
                    loaded(&theirs.bytes),
                    theirs.stderr,
                    ours.as_ref().ok().and_then(|bytes| loaded(bytes)),
                    linked.diagnostics.first(),
                ));
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_link_refuses_what_vlink_refuses() -> Result<(), Box<dyn Error>> {
    let Some(tool) = tool() else {
        return Ok(());
    };
    for (index, program) in REFUSED.iter().enumerate() {
        let inputs = inputs(program, &Options::unoptimised("t.s"));
        let objects: Vec<Vec<u8>> = inputs
            .iter()
            .map(write_hunk_object)
            .collect::<Result<_, _>>()?;
        let theirs = vlink(&tool, &objects, &format!("r{index}"))?;
        assert!(!theirs.success, "vlink accepted {program:?}");
        assert!(link(&inputs).has_errors(), "{program:?}");
    }
    Ok(())
}

/// Runs a linked program where vamos is available, which proves relocations load.
#[test]
fn test_link_result_runs_under_vamos() -> Result<(), Box<dyn Error>> {
    let Some(vamos) = std::env::var_os("RASM_VAMOS") else {
        eprintln!("skipping: RASM_VAMOS not set");
        return Ok(());
    };
    let program = [
        concat!(
            " xref add,value\n bsr add\n move.l value,d1\n add.l d1,d0\n",
            " cmp.l #49,d0\n beq ok\n moveq #1,d0\n rts\nok: moveq #0,d0\n rts\n",
        ),
        " xdef add\nadd: moveq #7,d0\n rts\n section d,data\n xdef value\nvalue: dc.l 42\n",
    ];
    let linked = link(&inputs(&program, &Options::new("t.s")));
    let path = std::env::temp_dir().join(format!("rasm-linked-{}", std::process::id()));
    fs::write(&path, write_hunk_exe(&linked)?)?;
    let run = Command::new(vamos).arg(&path).output();
    let _ = fs::remove_file(&path);
    assert_eq!(run?.status.code(), Some(0));
    Ok(())
}

#[test]
fn test_link_undefined_symbol_names_the_use() {
    let inputs = inputs(
        &[" nop", " xref gone\n jsr gone"],
        &Options::unoptimised("t.s"),
    );
    let linked = link(&inputs);
    let error = linked
        .diagnostics
        .iter()
        .find(|d| d.code == "undefined-symbol");
    let span = error.and_then(|error| error.span);
    assert_eq!(
        span.map(|span| (linked.files[span.file as usize].as_str(), span.line)),
        Some(("f1.s", 2))
    );
}

#[test]
fn test_link_keeps_lines_and_symbols_of_every_file() {
    let inputs = inputs(
        &[" nop\nlocal: nop", "there: rts"],
        &Options::unoptimised("t.s"),
    );
    let linked = link(&inputs);
    let entry = linked.line_for_address(0, 4);
    assert_eq!(
        entry.map(|entry| (entry.span.file, entry.span.line)),
        Some((1, 1))
    );
    let there = linked.symbols.iter().find(|symbol| symbol.name == "there");
    assert_eq!(there.map(|symbol| symbol.value), Some(4));
}
