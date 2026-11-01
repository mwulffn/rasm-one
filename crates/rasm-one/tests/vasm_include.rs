//! Compares `include` and `incbin` handling with vasm -no-opt.

use std::error::Error;

use rasm_one::{assemble, write_bin, Assembly, MemoryProvider, Options, Target};
use rasm_oracle::{Format, Vasm};

const FILES: &[(&str, &[u8])] = &[
    ("a.i", b" nop\n"),
    ("sub/b.i", b" rts\n"),
    ("sub/rel.i", b" include \"c.i\"\n"),
    ("sub/c.i", b" dc.w 2\n"),
    ("c.i", b" dc.w 9\n"),
    ("sub/nearby.i", b" include \"only_in_sub.i\"\n"),
    ("sub/only_in_sub.i", b" dc.w 4\n"),
    ("other/o.i", b" dc.w 7\n"),
    ("self.i", b" include \"self.i\"\n"),
    ("cyc1.i", b" include \"cyc2.i\"\n"),
    ("cyc2.i", b" include \"cyc1.i\"\n"),
    ("err.i", b" bogus\n"),
    ("lab.i", b"lab: nop\n"),
    ("e.i", b" nop\n end\n rts\n"),
    ("d.bin", b"ABCDEFGH"),
    ("nonl.i", b" dc.w 5"),
    ("crlf.i", b" nop\r\n rts\r\n"),
    ("latin1.i", b" dc.b \"\xe6\xf8\xe5\"\n"),
];

const ACCEPTED: &[&str] = &[
    " include \"a.i\"",
    " include 'a.i'",
    " include a.i",
    " INCLUDE \"a.i\"",
    " include \"a.i\" comment",
    " include \"sub/b.i\"",
    " include \"sub\\b.i\"",
    " include \"sub/rel.i\"",
    " include \"lab.i\"\n bra lab",
    " include \"a.i\"\n include \"a.i\"",
    " include \"e.i\"\n dc.w 3",
    " include \"nonl.i\"\n dc.w 3",
    " include \"crlf.i\"",
    " include \"latin1.i\"",
    "l: include \"a.i\"\n dc.l l",
    " dc.b 1\nl: include \"a.i\"\n dc.l l",
    " incbin \"d.bin\"",
    " incbin 'd.bin'",
    " incbin d.bin",
    " incbin \"d.bin\",2",
    " incbin \"d.bin\",2,3",
    " incbin \"d.bin\",8",
    " incbin \"d.bin\",2,100",
    " incbin \"d.bin\",2,-1",
    " incbin \"d.bin\",0,0",
    " dc.b 1\n incbin \"d.bin\"\n nop",
    "l: incbin \"d.bin\"\nm: dc.b m-l",
    "n=2\n incbin \"d.bin\",n",
    " section b,bss\n incbin \"d.bin\"",
    " incdir \"other\"\n include \"o.i\"",
    " incdir other\n include \"o.i\"",
    " incdir \"other/\"\n include \"o.i\"",
    " incdir \"sub\"\n incbin \"../d.bin\"",
    " incdir \"other\"\n include \"c.i\"",
    " incdir \"sub\"\n include \"c.i\"",
    " incdir \"missing\"\n nop",
];

const REJECTED: &[&str] = &[
    " include \"sub/nearby.i\"",
    " include \"missing.i\"",
    " include \"err.i\"",
    " include \"lab.i\"\n include \"lab.i\"",
    " include.w \"a.i\"",
    " incbin \"d.bin\",9",
    " incbin \"d.bin\",-1",
    " incbin \"missing.bin\"",
    " incbin \"d.bin\",n\nn=2",
    " incdir",
];

/// vasm ignores text after the file name, treats a missing name as doing
/// nothing, and doesn't return from an include cycle. We report all of these.
const STRICTER_THAN_VASM: &[&str] = &[
    " include \"a.i\",x",
    " include",
    " incbin",
    " include \"self.i\"",
    " include \"cyc1.i\"",
];

fn ours(source: &str) -> Assembly {
    let mut files = MemoryProvider::new();
    files.insert("t.s", source);
    for (path, contents) in FILES {
        files.insert(*path, contents.to_vec());
    }
    let mut options = Options::unoptimised("t.s");
    options.target = Target::RawBinary;
    assemble(&options, &files)
}

#[test]
fn test_include_accepted_sources_match_vasm_bytes() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in ACCEPTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble_with_files(&source, "68000", Format::Bin, FILES)?;
        assert!(
            theirs.success,
            "vasm rejected {source:?}: {}",
            theirs.stderr
        );
        let assembly = ours(&source);
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
fn test_include_rejected_sources_match_vasm_rejection() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let mut failures = Vec::new();
    for source in REJECTED {
        let source = format!("{source}\n");
        let theirs = vasm.assemble_with_files(&source, "68000", Format::Bin, FILES)?;
        assert!(!theirs.success, "vasm accepted {source:?}");
        if !ours(&source).has_errors() {
            failures.push(source);
        }
    }
    assert!(failures.is_empty(), "we accepted {failures:?}");
    Ok(())
}

#[test]
fn test_include_stricter_than_vasm_sources_are_errors() {
    for source in STRICTER_THAN_VASM {
        assert!(ours(&format!("{source}\n")).has_errors(), "{source:?}");
    }
}

#[test]
fn test_include_searches_main_source_directory() {
    let mut files = MemoryProvider::new();
    files.insert("src/t.s", " include \"a.i\"\n");
    files.insert("src/a.i", " nop\n");
    let mut options = Options::unoptimised("src/t.s");
    options.target = Target::RawBinary;
    let assembly = assemble(&options, &files);
    assert_eq!(write_bin(&assembly), Ok(vec![0x4E, 0x71]));
    assert_eq!(assembly.files[1], "src/a.i");
}

#[test]
fn test_include_dirs_option_searches_directory() {
    let mut files = MemoryProvider::new();
    files.insert("t.s", " include \"o.i\"\n");
    files.insert("other/o.i", " nop\n");
    let mut options = Options::unoptimised("t.s");
    options.target = Target::RawBinary;
    options.include_dirs = vec!["other".to_string()];
    assert_eq!(write_bin(&assemble(&options, &files)), Ok(vec![0x4E, 0x71]));
}
