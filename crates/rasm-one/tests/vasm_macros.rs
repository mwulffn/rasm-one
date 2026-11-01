//! Compares conditionals, macros, `rept` and local labels with vasm -no-opt.

use std::error::Error;

use rasm_one::{assemble, write_bin, write_hunk_exe, Assembly, MemoryProvider, Options, Target};
use rasm_oracle::{Format, Vasm};

const M: &str = "m macro\n dc.b \\1\n endm\n";

const ACCEPTED: &[&str] = &[
    // Conditionals.
    " if 1\n dc.b 1\n endif",
    " if 0\n dc.b 1\n endif",
    " if 0\n dc.b 1\n else\n dc.b 2\n endif",
    " if 1\n dc.b 1\n else\n dc.b 2\n endc",
    " ifne 1\n dc.b 1\n endc",
    " ifeq 0\n dc.b 1\n endc",
    " ifgt 1\n dc.b 1\n endc",
    " ifge 0\n dc.b 1\n endc",
    " iflt -1\n dc.b 1\n endc",
    " ifle 0\n dc.b 1\n endc",
    " ifgt 0\n dc.b 1\n endc",
    "a=1\n ifd a\n dc.b 1\n endc",
    " ifd a\n dc.b 1\n endc",
    " ifnd a\n dc.b 1\n endc",
    " ifd a\n dc.b 1\n endc\na=1",
    "a=1\n ifnd a\n dc.b 1\n else\n dc.b 2\n endc",
    " ifc \"a\",\"a\"\n dc.b 1\n endc",
    " ifc \"a\",\"b\"\n dc.b 1\n endc",
    " ifnc \"a\",\"b\"\n dc.b 1\n endc",
    " ifb\n dc.b 1\n endc",
    " ifb x\n dc.b 1\n endc",
    " ifnb x\n dc.b 1\n endc",
    " if 1\n if 0\n dc.b 1\n else\n dc.b 2\n endc\n else\n dc.b 3\n endc",
    " if 0\n if 1\n dc.b 1\n else\n dc.b 2\n endc\n else\n dc.b 3\n endc",
    " if 0\n bogus !!\n endc",
    "l: if 1\n dc.b 1\n endc\n dc.l l",
    " IF 1\n ENDC",
    " if 1 ;c\n dc.b 1\n endc ;c",
    " if 0\n elseif\n dc.b 1\n endc",
    " if 1\n dc.b 2\n elseif\n dc.b 1\n endc",
    " if 0\n elif 0\n dc.b 3\n elseif\n dc.b 1\n endc",
    " if 1\n dc.b 3\n elseif\n dc.b 1\n endc",
    " if 0\n dc.b 1\n elif 1\n dc.b 2\n endc",
    " if 1\n dc.b 1\n elif 1\n dc.b 2\n else\n dc.b 3\n endc",
    " if 0\n dc.b 1\n elif 0\n dc.b 2\n else\n dc.b 3\n endc",
    // rept.
    " rept 2\n dc.b 1\n endr",
    " rept 0\n dc.b 1\n endr",
    " rept -1\n dc.b 1\n endr",
    " rept 2\n rept 2\n dc.b 1\n endr\n endr",
    "n=2\n rept n\n dc.b 1\n endr",
    " rept 2\n dc.b REPTN\n endr",
    " rept 2\n rept 2\n dc.b REPTN\n endr\n dc.b REPTN\n endr",
    " rept 2\n dc.b 1\n endr ;x",
    " rept 2 ;c\n dc.b 1\n endr",
    "a set 0\n rept 3\na set a+1\n dc.b a\n endr",
    " rept 1\n if 0\n endr\n endc",
    " if 1\n rept 2\n dc.b 1\n endr\n endc",
    "l: rept 2\n dc.b 1\n endr\n dc.l l",
    // Macro forms.
    "m: macro\n nop\n endm\n m",
    " macro m\n nop\n endm\n m",
    "m macro\n nop\n endm",
    "m macro\n dc.b \\1,\\2\n endm\n m 1,2",
    "m macro\n dc.b 9\n endm\n m 1,2,3",
    "m macro\n dc.\\0 1\n endm\n m.w",
    "m macro\n dc.\\0 1\n endm\n m",
    "m macro\n move.\\0 d0,d1\n endm\n m.l",
    "m macro\n dc.b NARG\n endm\n m 1,2\n m",
    "m macro\n dc.b \\#\n endm\n m 1,2\n m",
    "m macro\nl\\@: dc.l l\\@\n endm\n m\n m",
    "m macro\n dc.b \"\\@\"\n endm\n m\n m",
    "n macro\n dc.b \"\\@\"\n endm\nm macro\n dc.b \"\\@\"\n n\n dc.b \"\\@\"\n endm\n m\n m",
    "m macro\nx\\@ nop\n endm\n m",
    "m macro\n dc.b \\9\n endm\n m 1,2,3,4,5,6,7,8,9",
    "m macro\n dc.b \\11\n endm\n m 1,2,3,4,5,6,7,8,9,10,11",
    // Macro arguments.
    "m macro\n dc.b \"\\1\"\n endm\n m abc",
    "m macro\n move.l \\1,d0\n endm\n m (a0,d1.w)",
    "m macro\n dc.b \\1,\\2\n endm\n m 1, 2",
    "m macro\n dc.b \\2\n endm\n m ,5",
    "m macro\n dc.b 1\\2\n endm\n m 1,2",
    "m macro\n dc.b \\1\\2\n endm\n m 1,2",
    "m macro\n dc.b $\\1\n endm\n m ff",
    "m macro\n dc.\\1 1\n endm\n m w",
    "m macro\n \\1 \\2,d0\n endm\n m move,d1",
    "m macro\n dc.b \\<a>\n endm\na=3\n m",
    "m macro\n ds.b 1\n \\1\n endm\n m nop",
    "m macro\n nop ; see \\z\n* full \\z\n endm\n m",
    // Macros with other blocks.
    "m macro\n ifc \"\\1\",\"\"\n dc.b 1\n else\n dc.b 2\n endc\n endm\n m\n m x",
    "m macro\n ifb \\1\n dc.b 1\n endc\n endm\n m\n m x",
    "m macro\n ifnb \\1\n dc.b 1\n endc\n endm\n m\n m x",
    "m macro\n if \\1\n dc.b 1\n mexit\n endc\n dc.b 2\n endm\n m 1\n m 0",
    "m macro\n rept 2\n dc.b \\1\n endr\n endm\n m 3",
    "n macro\n dc.b \\1\n endm\nm macro\n n \\1\n n 7\n endm\n m 3",
    "n macro\n dc.b NARG\n endm\nm macro\n n 1,2,3\n dc.b NARG\n endm\n m",
    "m macro\n if \\1\n dc.b \\1\n m \\1-1\n endc\n endm\n m 3",
    "m macro\n nop\n endm\nm macro\n rts\n endm\n m",
    "nop macro\n dc.b 1\n endm\n nop",
    "m macro\n endm\n m\nl: m\n dc.l l",
    "m macro\n dc.b 1\n endm\n dc.b 2\nl: m\n dc.l l",
    "m macro\n nop\n endm\n dc.b 2\nl: m\n dc.l l",
    "m macro\n nop ;c\n* full\n endm\n m",
    "m macro\n dc.b 1\n endm\n if 0\n m\n endc",
    "m macro\n.l: dc.l .l\n endm\na: m\nb: m",
    "m macro\n.l\\@: dc.l .l\\@\n endm\n m\n m",
    // Local labels.
    ".l: nop\n bra .l",
    "a: nop\n.l: nop\nb: nop\n.l: nop\n bra .l",
    "l$: nop\n bra l$",
    "1$: nop\n bra 1$",
    "a:\n1$: nop\nb:\n1$: nop\n dc.l 1$",
    " .l: nop",
    ".l nop\n dc.l .l",
    ".l=4\n dc.b .l",
    ".l equ 4\n dc.b .l",
    "a:\n.l set 4\nb:\n.l set 5\n dc.b .l",
    " dc.l .l\n.l:",
    "a:\n dc.l .l\n.l:\nb:\n.l:",
    "a=1\n.l:\n dc.l .l",
    "a set 1\n.l:\nb set 1\n dc.l .l",
    "a:\n.l:\n section x,data\n",
    "a:\n.l: nop\nb equ 1\n dc.l .l",
    "a:\n.l: nop\n rs.b 1\nb rs.b 1\n dc.l .l",
    "a:\n.l: nop\nb reg d0\n dc.l .l",
    "a:\n.l: nop\nb equr d0\n dc.l .l",
    "l: nop\n.w: nop\n move.w .w,d0\n move.w .w.w,d0",
];

const REJECTED: &[&str] = &[
    "m macro\n rept 3\n if REPTN=1\n mexit\n endc\n dc.b \\1\n endr\n dc.b 8\n endm\n m 3\n dc.b 9",
    "a:\n.l: nop\nb equ 1\n.l: nop",
    " if 0\nl: nop\n endc\n dc.l l",
    " if 1",
    " endc",
    " else",
    " if a\n endc\na=1",
    " if",
    " elseif 1",
    " rept n\n dc.b 1\n endr\nn=1",
    " rept 2\nl: nop\n endr",
    " rept 2\n dc.b 1",
    " endr",
    " rept",
    " rept 2\n dc.b \\+\n endr",
    " if 0\n rept 2\n endc\n endr",
    "m macro\n dc.b \\1\n endm\n M 1",
    "m macro\n dc.b \\1\n endm\n m",
    "m macro\n dc.b \\1,\\2\n endm\n m 1",
    "m macro\n dc.b \\0\n endm\n m.b 1",
    "m macro\n dc.b \\z\n endm\n m 1",
    "m macro a,b\n dc.b \\a,\\b\n endm\n m 1,2",
    "m macro\n mexit\n endm\n m\n mexit",
    "m macro\n m\n endm\n m",
    "m macro\nn macro\n dc.b 1\n endm\n endm\n m\n n",
    "dc macro\n dc.b 1\n endm\n dc",
    "l: m\nm macro\n nop\n endm",
    " m\nm macro\n nop\n endm",
    "m macro\n nop\n",
    " endm",
    " if 0\nm macro\n nop\n endm\n endc\n m",
    ".l: nop\n.l: nop",
    "a: nop\n.l: nop\nb: dc.l a\n.m dc.l .l",
    "a: .l: nop",
    " .l nop",
    "a:\n.x:\nb:\n dc.l .x",
    "m macro\n.l: dc.l .l\n endm\n m\n m",
    ".l: nop\n dc.l .L",
    "a.l: nop",
    "L: nop\n dc.l l",
    ".a.b: nop",
];

/// vasm accepts these with at most a warning. The dialect is strict, so they
/// are errors here.
const STRICTER_THAN_VASM: &[&str] = &[
    " if 0\n elseif 1\n dc.b 1\n endc",
    " if 0\n elseif 0\n dc.b 1\n endc",
    " if 1\n elseif 1\n dc.b 1\n endc",
    " if 1\n else\n else\n endc",
    " if 1,2\n endc",
    "m macro\n dc.b \\1\n endm\n m (1,2)",
    "m macro\n dc.b \"\\1\"\n endm\n m \"abc\"",
    "m macro\n dc.b \\$a\n endm\na=3\n m",
    "a\\@: nop",
];

/// Hunk executables with local labels and macro-generated symbols.
const HUNK_ACCEPTED: &[&str] = &[
    "a: nop\n.l: nop\nb: nop\n.l: rts",
    " section x,code\n.l:\n section y,code\n nop",
    " section x,code\nl:\n.l:\n section y,code\n nop",
    "a: nop\n1$: nop\nb: nop\nl$: rts",
    "m macro\nl\\@: dc.l l\\@\n endm\n m\n m",
    "m macro\n dc.b NARG\n endm\n m 1\nl: nop",
    " rept 2\n dc.b REPTN\n endr\nl: nop",
];

fn ours_bin(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    let mut options = Options::unoptimised("t.s");
    options.target = Target::RawBinary;
    assemble(&options, &files)
}

fn ours(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    assemble(&Options::unoptimised("t.s"), &files)
}

#[test]
fn test_macros_accepted_sources_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    let with_m: Vec<String> = [
        " m 1",
        " m 2\n m 3",
        " m 1 2",
        " m 1 ;c",
        " m <1,2>",
        " m \"a,b\"",
        " m 'a,b'",
    ]
    .iter()
    .map(|call| format!("{M}{call}"))
    .collect();
    let sources = ACCEPTED
        .iter()
        .copied()
        .chain(with_m.iter().map(String::as_str));
    for source in sources {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::Bin)?;
        if !theirs.success {
            failures.push(format!("vasm rejected {source:?}: {}", theirs.stderr));
            continue;
        }
        let assembly = ours_bin(&source);
        match write_bin(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {bytes:02x?}\n  vasm  {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}: {:?}", assembly.diagnostics)),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_macros_rejected_sources_match_vasm_rejection() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::Bin)?;
        if theirs.success {
            failures.push(format!("vasm accepted {source:?}"));
        }
        if !ours(&source).has_errors() {
            failures.push(format!("we accepted {source:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}

#[test]
fn test_macros_stricter_than_vasm_sources_are_errors() {
    for source in STRICTER_THAN_VASM {
        assert!(ours(&format!("{source}\n")).has_errors(), "{source:?}");
    }
}

#[test]
fn test_macros_hunk_output_matches_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in HUNK_ACCEPTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble(&source, "68000", Format::HunkExe)?;
        assert!(
            theirs.success,
            "vasm rejected {source:?}: {}",
            theirs.stderr
        );
        match write_hunk_exe(&ours(&source)) {
            Ok(bytes) if bytes == theirs.bytes => {}
            Ok(bytes) => failures.push(format!(
                "{source:?}\n  ours  {bytes:02x?}\n  vasm  {:02x?}",
                theirs.bytes
            )),
            Err(error) => failures.push(format!("{source:?}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    Ok(())
}
