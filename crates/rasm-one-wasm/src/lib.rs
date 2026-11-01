//! JavaScript bindings for rasm-one, for use in a browser IDE.
//!
//! ```js
//! const project = new Project();
//! project.add_file("main.s", new TextEncoder().encode(" nop\n"));
//! const result = project.assemble("main.s", "hunkexe");
//! const report = JSON.parse(result.report_json());
//! const bytes = result.bytes(); // Uint8Array, empty if there were errors
//! ```

mod report;

use std::rc::Rc;

use rasm_one::{
    assemble, link, write_bin, write_hunk_exe, write_hunk_exe_with, write_hunk_object,
    write_listing, Assembly, Cpu, FpuChoice, HunkExeOptions, MemoryProvider, Optimisations,
    Options, Target,
};
use wasm_bindgen::prelude::wasm_bindgen;

/// Source files held in memory.
#[wasm_bindgen]
#[derive(Default)]
pub struct Project {
    files: Rc<MemoryProvider>,
    include_dirs: Vec<String>,
    cpu: Option<Cpu>,
    has_coprocessor: bool,
    has_68851: bool,
    unoptimised: bool,
}

#[wasm_bindgen]
impl Project {
    /// Creates an empty project.
    #[wasm_bindgen(constructor)]
    pub fn new() -> Project {
        Project::default()
    }

    /// Adds or replaces a file.
    pub fn add_file(&mut self, path: &str, contents: &[u8]) {
        Rc::make_mut(&mut self.files).insert(path, contents);
    }

    /// Adds a directory to search for `include` and `incbin` files.
    pub fn add_include_dir(&mut self, directory: &str) {
        self.include_dirs.push(directory.to_string());
    }

    /// Chooses the CPU by number, such as 68020. Returns false if unknown.
    pub fn set_cpu(&mut self, number: u32) -> bool {
        self.cpu = Cpu::from_number(number);
        self.cpu.is_some()
    }

    /// Says that a 68881 or 68882 is available, alongside a 68020 or 68030.
    pub fn set_coprocessor(&mut self, present: bool) {
        self.has_coprocessor = present;
    }

    /// Says that a 68851 MMU is available, alongside a 68020 or 68030.
    pub fn set_68851(&mut self, present: bool) {
        self.has_68851 = present;
    }

    fn options(&self, main: &str, format: &str) -> Options {
        let mut options = Options::new(main);
        options.include_dirs = self.include_dirs.clone();
        options.target = if format == "bin" {
            Target::RawBinary
        } else {
            Target::Relocatable
        };
        if self.unoptimised {
            options.optimisations = Optimisations::off();
        }
        if let Some(cpu) = self.cpu {
            options.cpu = cpu;
        }
        options.mmu_68851 = self.has_68851;
        if self.has_coprocessor {
            options.fpu = FpuChoice::Coprocessor;
        }
        options
    }

    fn result(assembly: Assembly, output: Result<Vec<u8>, String>) -> AssemblyResult {
        let (bytes, output_error) = match output {
            Ok(bytes) => (bytes, None),
            Err(error) => (Vec::new(), Some(error)),
        };
        AssemblyResult {
            bytes,
            report: report::to_json(&assembly, output_error.as_deref()),
            assembly,
        }
    }

    /// Assembles each of `mains` and links them into a hunk executable.
    pub fn assemble_and_link(&self, mains: Vec<String>) -> AssemblyResult {
        let inputs: Vec<Assembly> = mains
            .iter()
            .map(|main| assemble(&self.options(main, "hunkexe"), self.files.as_ref()))
            .collect();
        let assembly = link(&inputs);
        let output = write_hunk_exe(&assembly).map_err(|error| error.to_string());
        Self::result(assembly, output)
    }

    /// Whether to optimise as vasm does by default, which is on unless this says otherwise.
    pub fn set_optimise(&mut self, on: bool) {
        self.unoptimised = !on;
    }

    /// Assembles `main` into `format`: `"hunkexe"`, `"hunkexe-kick1"` (which also
    /// loads on Kickstart 1.x), `"hunk"` or `"bin"`.
    pub fn assemble(&self, main: &str, format: &str) -> AssemblyResult {
        let options = self.options(main, format);
        let assembly = assemble(&options, self.files.as_ref());
        let output = match format {
            "hunkexe" => write_hunk_exe(&assembly).map_err(|error| error.to_string()),
            "hunkexe-kick1" => {
                let options = HunkExeOptions { kickstart1: true };
                write_hunk_exe_with(&assembly, options).map_err(|error| error.to_string())
            }
            "hunk" => write_hunk_object(&assembly).map_err(|error| error.to_string()),
            "bin" => write_bin(&assembly).map_err(|error| error.to_string()),
            other => Err(format!("unknown format {other}")),
        };
        Self::result(assembly, output)
    }
}

/// What an assembly produced.
#[wasm_bindgen]
pub struct AssemblyResult {
    bytes: Vec<u8>,
    report: String,
    assembly: Assembly,
}

#[wasm_bindgen]
impl AssemblyResult {
    /// The output file; empty if it could not be written.
    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.clone()
    }

    /// Diagnostics, symbols, sections and the line table, as JSON.
    pub fn report_json(&self) -> String {
        self.report.clone()
    }

    /// A human-readable listing, made when asked for since it costs time.
    pub fn listing(&self) -> String {
        write_listing(&self.assembly)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(source: &str) -> Project {
        let mut project = Project::new();
        project.add_file("t.s", source.as_bytes());
        project
    }

    #[test]
    fn test_assemble_writes_requested_format() {
        let result = project(" nop\n").assemble("t.s", "bin");
        assert_eq!(result.bytes(), [0x4E, 0x71]);
        let result = project(" nop\n").assemble("t.s", "hunkexe");
        assert_eq!(result.bytes()[..4], [0, 0, 3, 0xF3]);
        let result = project(" nop\n").assemble("t.s", "hunk");
        assert_eq!(result.bytes()[..4], [0, 0, 3, 0xE7]);
    }

    #[test]
    fn test_assemble_errors_give_report_and_no_bytes() {
        let result = project(" bogus\n").assemble("t.s", "bin");
        assert!(result.bytes().is_empty());
        assert!(result.report_json().contains("unknown-mnemonic"));
        assert!(result.report_json().contains("assembly has errors"));
    }

    #[test]
    fn test_assemble_unknown_format_is_reported() {
        let result = project(" nop\n").assemble("t.s", "elf");
        assert!(result.bytes().is_empty());
        assert!(result.report_json().contains("unknown format elf"));
    }

    #[test]
    fn test_assemble_uses_added_files_and_include_dirs() {
        let mut project = project(" include \"a.i\"\n");
        project.add_file("lib/a.i", b" rts\n");
        project.add_include_dir("lib");
        assert_eq!(project.assemble("t.s", "bin").bytes(), [0x4E, 0x75]);
    }

    #[test]
    fn test_listing_is_available() {
        let result = project(" nop\n").assemble("t.s", "bin");
        assert!(result.listing().contains("4E71"));
    }
}
