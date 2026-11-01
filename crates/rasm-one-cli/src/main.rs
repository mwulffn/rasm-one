//! Command-line front end for rasm-one.

use std::fs;

use anyhow::{anyhow, bail, Context, Result};
use rasm_one::{
    assemble, write_bin, write_hunk_exe_with, write_hunk_object, write_listing, Cpu, FileError,
    FileProvider, FpuChoice, HunkExeOptions, Optimisations, Options, Severity,
};

const USAGE: &str =
    "usage: rasm-one <source> -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-kick1hunks] [-m68000 ... -m68060] [-m68881 | -m68882 | -no-fpu] [-m68851] [-I<dir>]... [-L <listing>] [-no-opt | -opt-o+ | -opt-o- | -opt-o<rule>+ | -opt-o<rule>-]...";

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
    cpu: Cpu,
    fpu: FpuChoice,
    mmu_68851: bool,
    optimisations: Optimisations,
}

/// Handles what follows `-opt-o`: a sign, or a rule's number and a sign.
fn apply_optimisation_flag(
    optimisations: &mut Optimisations,
    option: &str,
    arg: &str,
) -> Result<()> {
    let (rule, on) = match (option.strip_suffix('+'), option.strip_suffix('-')) {
        (Some(rule), _) => (rule, true),
        (_, Some(rule)) => (rule, false),
        _ => bail!("unknown option {arg}\n{USAGE}"),
    };
    if !optimisations.enabled {
        *optimisations = Optimisations::all();
    }
    if rule.is_empty() {
        optimisations.set_all_rules(on);
    } else {
        let number = Optimisations::rule_named(rule)
            .ok_or_else(|| anyhow!("unknown option {arg}\n{USAGE}"))?;
        if !optimisations.set_rule(number, on) {
            eprintln!("warning: optimisation rule {number} doesn't exist yet, so {arg} is ignored");
        }
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args> {
    let (mut source, mut output) = (None, None);
    let mut format = Format::HunkExe;
    let mut include_dirs = Vec::new();
    let mut listing = None;
    let mut kickstart1 = false;
    let mut cpu = Cpu::default();
    let mut fpu = FpuChoice::default();
    let mut mmu_68851 = false;
    let mut optimisations = Optimisations::default();
    // As in vasm, this wins over any -opt- flag, wherever it is.
    let mut no_opt = false;
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
        } else if arg == "-m68881" || arg == "-m68882" {
            fpu = FpuChoice::Coprocessor;
        } else if arg == "-m68851" {
            mmu_68851 = true;
        } else if arg == "-no-opt" {
            no_opt = true;
        } else if let Some(option) = arg.strip_prefix("-opt-o") {
            apply_optimisation_flag(&mut optimisations, option, &arg)?;
        } else if arg == "-no-fpu" {
            fpu = FpuChoice::None;
        } else if let Some(found) = arg
            .strip_prefix("-m")
            .and_then(|number| number.parse().ok())
            .and_then(Cpu::from_number)
        {
            cpu = found;
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
            cpu,
            fpu,
            mmu_68851,
            optimisations: if no_opt {
                Optimisations::default()
            } else {
                optimisations
            },
        }),
        _ => bail!("{USAGE}"),
    }
}

fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let mut options = Options::new(args.source);
    options.include_dirs = args.include_dirs;
    options.cpu = args.cpu;
    options.fpu = args.fpu;
    options.mmu_68851 = args.mmu_68851;
    options.optimisations = args.optimisations;
    options.fixed_addresses = args.format == Format::Bin;
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
        let on = parse(&["a.s", "-o", "x", "-opt-o-", "-opt-o1+"])?.optimisations;
        assert!(on.enabled);
        let off = parse(&["-opt-o1+", "a.s", "-o", "x", "-no-opt"])?.optimisations;
        assert!(!off.enabled);
        assert!(parse(&["a.s", "-o", "x", "-opt-ox+"]).is_err());
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
    fn test_parse_args_cpu() -> Result<()> {
        assert_eq!(parse(&["a.s", "-o", "x", "-m68030"])?.cpu, Cpu::M68030);
        assert!(parse(&["a.s", "-o", "x", "-m68050"]).is_err());
        Ok(())
    }

    #[test]
    fn test_parse_args_rejects_bad_input() {
        assert!(parse(&["a.s"]).is_err());
        assert!(parse(&["a.s", "b.s", "-o", "x"]).is_err());
        assert!(parse(&["-x", "a.s", "-o", "x"]).is_err());
    }
}
