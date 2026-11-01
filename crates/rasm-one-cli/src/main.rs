//! Command-line front end for rasm-one.

use std::fs;

use anyhow::{bail, Context, Result};
use rasm_one::{
    assemble, write_bin, write_hunk_exe_with, write_hunk_object, write_listing, FileError,
    FileProvider, HunkExeOptions, Options, Severity,
};

const USAGE: &str =
    "usage: rasm-one <source> -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-kick1hunks] [-I<dir>]... [-L <listing>]";

/// Reads files from the native filesystem.
struct NativeProvider;

impl FileProvider for NativeProvider {
    fn read(&self, path: &str, _including_file: Option<&str>) -> Result<Vec<u8>, FileError> {
        fs::read(path).map_err(|error| match error.kind() {
            std::io::ErrorKind::NotFound => FileError::NotFound(path.to_string()),
            _ => FileError::Other(format!("{path}: {error}")),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Format {
    HunkExe,
    HunkObject,
    Bin,
}

struct Args {
    source: String,
    output: String,
    format: Format,
    include_dirs: Vec<String>,
    listing: Option<String>,
    kickstart1: bool,
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args> {
    let (mut source, mut output) = (None, None);
    let mut format = Format::HunkExe;
    let mut include_dirs = Vec::new();
    let mut listing = None;
    let mut kickstart1 = false;
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "-o" {
            output = Some(args.next().context("-o needs a file name")?);
        } else if arg == "-L" {
            listing = Some(args.next().context("-L needs a file name")?);
        } else if arg == "-kick1hunks" {
            kickstart1 = true;
        } else if arg == "-Fhunkexe" {
            format = Format::HunkExe;
        } else if arg == "-Fhunk" {
            format = Format::HunkObject;
        } else if arg == "-Fbin" {
            format = Format::Bin;
        } else if let Some(directory) = arg.strip_prefix("-I") {
            include_dirs.push(directory.to_string());
        } else if arg.starts_with('-') {
            bail!("unknown option {arg}\n{USAGE}");
        } else if source.replace(arg).is_some() {
            bail!("more than one source file\n{USAGE}");
        }
    }
    match (source, output) {
        (Some(source), Some(output)) => Ok(Args {
            source,
            output,
            format,
            include_dirs,
            listing,
            kickstart1,
        }),
        _ => bail!("{USAGE}"),
    }
}

fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let mut options = Options::new(args.source);
    options.include_dirs = args.include_dirs;
    let assembly = assemble(&options, &NativeProvider);
    if let Some(path) = &args.listing {
        let text = write_listing(&assembly, &NativeProvider);
        fs::write(path, text).with_context(|| format!("writing {path}"))?;
    }
    for diagnostic in &assembly.diagnostics {
        eprintln!("{}", assembly.render_diagnostic(diagnostic));
    }
    let errors = assembly
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    if errors > 0 {
        bail!("{errors} error(s)");
    }
    let bytes = match args.format {
        Format::HunkExe => write_hunk_exe_with(
            &assembly,
            HunkExeOptions {
                kickstart1: args.kickstart1,
            },
        )?,
        Format::HunkObject => write_hunk_object(&assembly)?,
        Format::Bin => write_bin(&assembly)?,
    };
    fs::write(&args.output, bytes).with_context(|| format!("writing {}", args.output))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args> {
        parse_args(args.iter().map(ToString::to_string))
    }

    #[test]
    fn test_parse_args_source_and_output() -> Result<()> {
        let args = parse(&["a.s", "-o", "a.bin"])?;
        assert_eq!(
            (args.source.as_str(), args.output.as_str()),
            ("a.s", "a.bin")
        );
        Ok(())
    }

    #[test]
    fn test_parse_args_format_defaults_to_hunk_exe() -> Result<()> {
        assert_eq!(parse(&["a.s", "-o", "x"])?.format, Format::HunkExe);
        assert_eq!(parse(&["a.s", "-o", "x", "-Fbin"])?.format, Format::Bin);
        Ok(())
    }

    #[test]
    fn test_parse_args_include_dirs_in_order() -> Result<()> {
        let args = parse(&["a.s", "-o", "x", "-Ione", "-Itwo"])?;
        assert_eq!(args.include_dirs, ["one", "two"]);
        Ok(())
    }

    #[test]
    fn test_parse_args_listing_file() -> Result<()> {
        let args = parse(&["a.s", "-o", "x", "-L", "a.lst"])?;
        assert_eq!(args.listing.as_deref(), Some("a.lst"));
        assert!(parse(&["a.s", "-o", "x", "-L"]).is_err());
        Ok(())
    }

    #[test]
    fn test_parse_args_rejects_bad_input() {
        assert!(parse(&["a.s"]).is_err());
        assert!(parse(&["a.s", "b.s", "-o", "x"]).is_err());
        assert!(parse(&["-x", "a.s", "-o", "x"]).is_err());
    }
}
