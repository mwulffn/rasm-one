//! Command-line front end for rasm-one.

use std::fs;

use anyhow::{bail, Context, Result};
use rasm_one::{
    assemble, link, write_bin, write_hunk_exe_with, write_hunk_object, write_listing, Cpu,
    FileError, FileProvider, FpuChoice, HunkExeOptions, Optimisations, Options, Rule, Severity,
    Target,
};

const USAGE: &str =
    "usage: rasm-one <source>... -o <output> [-Fhunkexe | -Fhunk | -Fbin] [-kick1hunks] [-m68000 ... -m68060] [-m68881 | -m68882 | -no-fpu] [-m68851] [-I<dir>]... [-L <listing>] [-linedebug [-noabspath]] [-no-opt | -opt-o+ | -opt-o- | -opt-o<rule>+ | -opt-o<rule>-]...";

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
    sources: Vec<String>,
    output: String,
    format: Format,
    include_dirs: Vec<String>,
    listing: Option<String>,
    kickstart1: bool,
    line_debug: bool,
    relative_paths: bool,
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
    if rule.is_empty() {
        optimisations.set_all_rules(on);
        return Ok(());
    }
    let is_letter = rule.len() == 1 && rule.bytes().all(|byte| byte.is_ascii_lowercase());
    if !is_letter && !rule.bytes().all(|byte| byte.is_ascii_digit()) {
        bail!("unknown option {arg}\n{USAGE}");
    }
    match Rule::named(rule) {
        Some(rule) => optimisations.set_rule(rule, on),
        None => eprintln!("warning: optimisation {rule} doesn't exist yet, so {arg} is ignored"),
    }
    Ok(())
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args> {
    let mut sources = Vec::new();
    let mut output = None;
    let mut format = Format::HunkExe;
    let mut include_dirs = Vec::new();
    let mut listing = None;
    let mut kickstart1 = false;
    let mut line_debug = false;
    let mut relative_paths = false;
    let mut cpu = Cpu::default();
    let mut fpu = FpuChoice::default();
    let mut mmu_68851 = false;
    // As in vasm, everything is on unless the command line says otherwise.
    let mut optimisations = Optimisations::all();
    // As in vasm, this wins over any -opt- flag, wherever it is.
    let mut no_opt = false;
    let mut args = args;
    while let Some(arg) = args.next() {
        if arg == "-o" {
            output = Some(args.next().context("-o needs a file name")?);
        } else if arg == "-L" {
            listing = Some(args.next().context("-L needs a file name")?);
        } else if arg == "-linedebug" {
            line_debug = true;
        } else if arg == "-noabspath" {
            relative_paths = true;
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
        } else {
            sources.push(arg);
        }
    }
    match output {
        Some(output) if !sources.is_empty() => {
            if sources.len() > 1 && format != Format::HunkExe {
                bail!("only an executable can be made from more than one source\n{USAGE}");
            }
            Ok(Args {
                sources,
                output,
                format,
                include_dirs,
                listing,
                kickstart1,
                line_debug,
                relative_paths,
                cpu,
                fpu,
                mmu_68851,
                optimisations: if no_opt {
                    Optimisations::off()
                } else {
                    optimisations
                },
            })
        }
        _ => bail!("{USAGE}"),
    }
}

fn main() -> Result<()> {
    let args = parse_args(std::env::args().skip(1))?;
    let mut options = Options::new("");
    options.include_dirs = args.include_dirs;
    options.cpu = args.cpu;
    options.fpu = args.fpu;
    options.mmu_68851 = args.mmu_68851;
    options.optimisations = args.optimisations;
    options.line_debug = args.line_debug;
    options.target = if args.format == Format::Bin {
        Target::RawBinary
    } else {
        Target::Relocatable
    };
    let mut assemblies: Vec<_> = args
        .sources
        .iter()
        .map(|source| {
            let mut options = options.clone();
            options.main = source.clone();
            assemble(&options, &NativeProvider)
        })
        .collect();
    let mut assembly = match assemblies.len() {
        1 => assemblies.remove(0),
        _ => link(&assemblies),
    };
    if let Some(path) = &args.listing {
        let text = write_listing(&assembly);
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
    if args.line_debug && !args.relative_paths {
        // As in vasm, so that a debugger can find them.
        let directory = std::env::current_dir().context("finding the current directory")?;
        for file in &mut assembly.files {
            *file = directory.join(&*file).display().to_string();
        }
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
            (args.sources[0].as_str(), args.output.as_str()),
            ("a.s", "a.bin")
        );
        Ok(())
    }

    #[test]
    fn test_parse_args_format_defaults_to_hunk_exe() -> Result<()> {
        assert_eq!(parse(&["a.s", "-o", "x"])?.format, Format::HunkExe);
        assert_eq!(parse(&["a.s", "-o", "x", "-Fbin"])?.format, Format::Bin);
        let on = parse(&["a.s", "-o", "x", "-opt-o-", "-opt-o1+"])?.optimisations;
        assert!(on.enabled());
        assert_eq!(
            parse(&["a.s", "-o", "x"])?.optimisations,
            Optimisations::all()
        );
        let off = parse(&["-opt-o1+", "a.s", "-o", "x", "-no-opt"])?.optimisations;
        assert!(!off.enabled());
        assert!(parse(&["a.s", "-o", "x", "-opt-oxy+"]).is_err());
        assert_eq!(parse(&["a.s", "b.s", "-o", "x"])?.sources.len(), 2);
        assert!(parse(&["a.s", "b.s", "-o", "x", "-Fhunk"]).is_err());
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
        assert!(parse(&["-x", "a.s", "-o", "x"]).is_err());
    }
}
