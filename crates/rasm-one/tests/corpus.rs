//! Assembles whole real-world sources and compares them with vasm -no-opt.
//! `RASM_CORPUS` comes from `scripts/fetch-corpus.sh`. Without it this skips
//! locally and fails in CI.
//!
//! Many of these sources need Amiga system includes that aren't in the corpus, so
//! vasm rejects them too. Only files vasm accepts say anything about rasm-one.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use rasm_one::{assemble, write_hunk_object, FileError, FileProvider, Options};
use rasm_oracle::Vasm;

/// Fewer than this means the corpus or the test has gone wrong.
const MINIMUM_MATCHES: usize = 90;

/// Files vasm accepts that are errors here, reviewed, with the error's code.
const EXPECTED_REJECTIONS: &[(&str, &str)] = &[
    // `sp equ $c00` makes a register name a symbol.
    ("ACCSources/Source17/Trev/hw.s", "reserved-name"),
];

/// Codes that are accepted for any file, because rasm-one rejects them by design.
fn rejected_by_design(code: &str) -> bool {
    // `opt` could change what a source means, so it isn't silently ignored.
    code == "unsupported-opt"
}

/// Not really source: a binary file that happens to be named `.s`.
const NOT_SOURCE: &[&str] = &["ACCSources/Source23/M.Bateson/Graphics/alien.s"];

struct Directory {
    dir: PathBuf,
    root: PathBuf,
}

impl FileProvider for Directory {
    fn read(&self, path: &str, _including_file: Option<&str>) -> Result<Vec<u8>, FileError> {
        let base = if Path::new(path).is_absolute() {
            PathBuf::from(path)
        } else {
            self.dir.join(path)
        };
        fs::read(base).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory => {
                FileError::NotFound(path.to_string())
            }
            _ => FileError::Other(format!("{path}: {error}")),
        })
    }
}

fn sources_in(dir: &Path, found: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            sources_in(&path, found)?;
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| {
                extension.eq_ignore_ascii_case("s") || extension.eq_ignore_ascii_case("asm")
            })
        {
            found.push(path);
        }
    }
    Ok(())
}

#[test]
fn test_corpus_matches_vasm() -> Result<(), Box<dyn Error>> {
    let Some(vasm) = Vasm::from_env()? else {
        return Ok(());
    };
    let Some(corpus) = std::env::var_os("RASM_CORPUS").map(PathBuf::from) else {
        assert!(
            std::env::var_os("CI").is_none(),
            "RASM_CORPUS is not set but CI requires it"
        );
        eprintln!("skipping: RASM_CORPUS not set (see scripts/fetch-corpus.sh)");
        return Ok(());
    };
    let mut files = Vec::new();
    sources_in(&corpus, &mut files)?;
    files.sort();

    let (mut matches, mut skipped) = (0, 0);
    let mut failures = Vec::new();
    for path in &files {
        let name = path
            .strip_prefix(&corpus)?
            .to_string_lossy()
            .replace('\\', "/");
        let short = name.split_once('/').map_or(name.as_str(), |(_, rest)| rest);
        if NOT_SOURCE.contains(&short) {
            continue;
        }
        let (Some(dir), Some(file)) = (path.parent(), path.file_name().and_then(|f| f.to_str()))
        else {
            continue;
        };
        let include_dir = corpus.join(name.split('/').next().unwrap_or(""));
        let include = format!("-I{}", include_dir.display());
        // -x makes undefined symbols errors, as they are here.
        let theirs = vasm.assemble_file(dir, file, "68000", &["-x", &include])?;
        if !theirs.success {
            skipped += 1;
            continue;
        }
        let provider = Directory {
            dir: dir.to_path_buf(),
            root: include_dir.clone(),
        };
        let mut options = Options::new(file);
        options.include_dirs = vec![provider.root.display().to_string()];
        let assembly = assemble(&options, &provider);
        match write_hunk_object(&assembly) {
            Ok(bytes) if bytes == theirs.bytes => matches += 1,
            Ok(_) => failures.push(format!("{short}: bytes differ")),
            Err(_) => {
                let code = assembly
                    .diagnostics
                    .iter()
                    .find(|d| d.severity == rasm_one::Severity::Error)
                    .map_or("output", |d| d.code);
                let expected = EXPECTED_REJECTIONS.contains(&(short, code));
                if !expected && !rejected_by_design(code) {
                    failures.push(format!("{short}: {:?}", assembly.diagnostics.first()));
                }
            }
        }
    }
    eprintln!(
        "{matches} files match, vasm rejected {skipped} of {}",
        files.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(matches >= MINIMUM_MATCHES, "only {matches} files matched");
    Ok(())
}
