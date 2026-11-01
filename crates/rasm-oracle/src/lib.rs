//! Runs an unmodified vasm as a black-box reference for differential tests.
//!
//! vasm's path comes from `RASM_VASM`; `scripts/fetch-vasm.sh` builds it.

use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

use thiserror::Error;

/// Name given to the source file, which vasm embeds in hunk objects.
pub const SOURCE_NAME: &str = "t.s";

/// Failure to run vasm at all, as opposed to vasm rejecting the source.
#[derive(Debug, Error)]
pub enum OracleError {
    /// Temporary files could not be written or read, or vasm could not start.
    #[error("vasm I/O failure: {0}")]
    Io(#[from] std::io::Error),
    /// CI must not silently skip differential tests.
    #[error("RASM_VASM is not set but CI requires it")]
    MissingInCi,
}

/// vasm output module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// Raw binary (`-Fbin`).
    Bin,
    /// Amiga hunk object (`-Fhunk`).
    HunkObject,
    /// Amiga hunk executable (`-Fhunkexe`).
    HunkExe,
}

impl Format {
    fn flag(self) -> &'static str {
        match self {
            Format::Bin => "-Fbin",
            Format::HunkObject => "-Fhunk",
            Format::HunkExe => "-Fhunkexe",
        }
    }
}

/// Result of a vasm run that started successfully.
#[derive(Debug)]
pub struct Output {
    /// True if vasm exited successfully.
    pub success: bool,
    /// Bytes written by vasm; empty if it produced no file.
    pub bytes: Vec<u8>,
    /// vasm's diagnostics.
    pub stderr: String,
}

/// A vasm binary.
#[derive(Debug, Clone)]
pub struct Vasm {
    path: PathBuf,
}

impl Vasm {
    /// Uses the binary at `path`.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    /// Reads `RASM_VASM`. Returns `Ok(None)` if unset, so tests can skip
    /// locally, and an error if unset under CI.
    pub fn from_env() -> Result<Option<Self>, OracleError> {
        match std::env::var_os("RASM_VASM") {
            Some(path) => Ok(Some(Self::new(path))),
            None if std::env::var_os("CI").is_some() => Err(OracleError::MissingInCi),
            None => {
                eprintln!("skipping: RASM_VASM not set (see scripts/fetch-vasm.sh)");
                Ok(None)
            }
        }
    }

    /// Assembles `source` for `cpu` (such as `"68000"`) with `-no-opt`.
    pub fn assemble(&self, source: &str, cpu: &str, format: Format) -> Result<Output, OracleError> {
        self.assemble_with_files(source, cpu, format, &[])
    }

    /// Assembles an existing file to a hunk object, from its own directory. As with
    /// [`Vasm::assemble_with_args`], an `-opt-` argument replaces `-no-opt`.
    pub fn assemble_file(
        &self,
        dir: &Path,
        file: &str,
        cpu: &str,
        args: &[&str],
    ) -> Result<Output, OracleError> {
        let out_dir = unique_temp_dir()?;
        let out_file = out_dir.join("out");
        let run = Command::new(&self.path)
            .current_dir(dir)
            .args(["-quiet", "-maxerrors=1", "-Fhunk"])
            .args(no_opt_unless_optimising(args))
            .arg(format!("-m{cpu}"))
            .args(args)
            .arg("-o")
            .arg(&out_file)
            .arg(file)
            .output();
        let bytes = fs::read(&out_file).unwrap_or_default();
        // Best effort: a leftover temp dir must not fail the test.
        let _ = fs::remove_dir_all(&out_dir);
        let out = run?;
        Ok(Output {
            success: out.status.success(),
            bytes,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }

    /// Like [`Vasm::assemble`], with extra command-line arguments. If any of them
    /// is an `-opt-` one, `-no-opt` is left out.
    pub fn assemble_with_args(
        &self,
        source: &str,
        cpu: &str,
        format: Format,
        args: &[&str],
    ) -> Result<Output, OracleError> {
        let dir = unique_temp_dir()?;
        let result = self.run_in(&dir, source, cpu, format, args);
        // Best effort: a leftover temp dir must not fail the test.
        let _ = fs::remove_dir_all(&dir);
        result
    }

    /// Like [`Vasm::assemble`], with extra files beside the source, given as
    /// relative paths and contents.
    pub fn assemble_with_files(
        &self,
        source: &str,
        cpu: &str,
        format: Format,
        files: &[(&str, &[u8])],
    ) -> Result<Output, OracleError> {
        let dir = unique_temp_dir()?;
        let result =
            write_files(&dir, files).and_then(|()| self.run_in(&dir, source, cpu, format, &[]));
        // Best effort: a leftover temp dir must not fail the test.
        let _ = fs::remove_dir_all(&dir);
        result
    }

    /// Like [`Vasm::assemble_with_args`], but keeps vasm's answer on disk, by a hash of the
    /// vasm binary and everything given to it, and reuses it next time. vasm has no other
    /// inputs, so the answer can't go stale. `RASM_VASM_CACHE` names the directory; without
    /// it, vasm always runs.
    pub fn assemble_cached(
        &self,
        source: &str,
        cpu: &str,
        format: Format,
        args: &[&str],
    ) -> Result<Output, OracleError> {
        let Some(directory) = std::env::var_os("RASM_VASM_CACHE").map(PathBuf::from) else {
            return self.assemble_with_args(source, cpu, format, args);
        };
        let mut hasher = DefaultHasher::new();
        fs::read(&self.path)?.hash(&mut hasher);
        (source, cpu, format.flag(), args).hash(&mut hasher);
        let entry = directory.join(format!("{:016x}", hasher.finish()));
        if let Some(output) = fs::read(&entry).ok().and_then(|data| decode_output(&data)) {
            return Ok(output);
        }
        let output = self.assemble_with_args(source, cpu, format, args)?;
        fs::create_dir_all(&directory)?;
        // Written whole and then renamed, as other threads may want the same entry.
        let partial = directory.join(format!(
            "{:016x}.{:?}.tmp",
            hasher.finish(),
            std::thread::current().id()
        ));
        fs::write(&partial, encode_output(&output))?;
        fs::rename(&partial, &entry)?;
        Ok(output)
    }

    /// Like [`Vasm::assemble_with_files`], with extra command-line arguments as in
    /// [`Vasm::assemble_with_args`].
    pub fn assemble_with_files_and_args(
        &self,
        source: &str,
        cpu: &str,
        format: Format,
        files: &[(&str, &[u8])],
        args: &[&str],
    ) -> Result<Output, OracleError> {
        let dir = unique_temp_dir()?;
        let result =
            write_files(&dir, files).and_then(|()| self.run_in(&dir, source, cpu, format, args));
        // Best effort: a leftover temp dir must not fail the test.
        let _ = fs::remove_dir_all(&dir);
        result
    }

    fn run_in(
        &self,
        dir: &Path,
        source: &str,
        cpu: &str,
        format: Format,
        args: &[&str],
    ) -> Result<Output, OracleError> {
        fs::write(dir.join(SOURCE_NAME), source)?;
        let out = Command::new(&self.path)
            .current_dir(dir)
            .args(["-quiet", "-maxerrors=0"])
            .args(no_opt_unless_optimising(args))
            .arg(format!("-m{cpu}"))
            .arg(format.flag())
            .args(args)
            .args(["-o", "out", SOURCE_NAME])
            .output()?;
        let bytes = match fs::read(dir.join("out")) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(e) => return Err(e.into()),
        };
        Ok(Output {
            success: out.status.success(),
            bytes,
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        })
    }
}

fn write_files(dir: &Path, files: &[(&str, &[u8])]) -> Result<(), OracleError> {
    for (path, contents) in files {
        let full = dir.join(path);
        if let Some(parent) = full.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(full, contents)?;
    }
    Ok(())
}

fn unique_temp_dir() -> Result<PathBuf, std::io::Error> {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("rasm-oracle-{}-{id}", std::process::id()));
    fs::create_dir_all(&dir)?;
    Ok(dir)
}

fn encode_output(output: &Output) -> Vec<u8> {
    let mut data = vec![u8::from(output.success)];
    data.extend_from_slice(&(output.bytes.len() as u64).to_be_bytes());
    data.extend_from_slice(&output.bytes);
    data.extend_from_slice(output.stderr.as_bytes());
    data
}

fn decode_output(data: &[u8]) -> Option<Output> {
    let (success, rest) = data.split_first()?;
    let (length, rest) = rest.split_first_chunk::<8>()?;
    let length = usize::try_from(u64::from_be_bytes(*length)).ok()?;
    let bytes = rest.get(..length)?.to_vec();
    let stderr = String::from_utf8_lossy(rest.get(length..)?).into_owned();
    Some(Output {
        success: *success != 0,
        bytes,
        stderr,
    })
}

/// Asking for optimisation rules replaces the usual `-no-opt`.
fn no_opt_unless_optimising(args: &[&str]) -> &'static [&'static str] {
    if args.iter().any(|arg| arg.starts_with("-opt-")) {
        &[]
    } else {
        &["-no-opt"]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decode_output_reads_what_encode_output_wrote() {
        let output = Output {
            success: true,
            bytes: vec![0x4E, 0x71],
            stderr: "warning\n".to_string(),
        };
        let decoded = decode_output(&encode_output(&output));
        assert!(decoded.is_some_and(|decoded| decoded.success
            && decoded.bytes == output.bytes
            && decoded.stderr == output.stderr));
    }

    #[test]
    fn test_decode_output_refuses_a_cut_short_entry() {
        assert!(decode_output(&[1, 0, 0]).is_none());
    }
}
