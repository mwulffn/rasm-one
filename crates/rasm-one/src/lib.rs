//! rasm-one: a 68k assembler library using Motorola syntax.
//!
//! The library performs no direct filesystem access, so it can also run in a
//! browser through WASM. All files come through a [`FileProvider`]:
//! [`MemoryProvider`] holds them in memory, and a host can implement the trait
//! for anything else.
//!
//! [`assemble`] never fails outright. It returns an [`Assembly`] holding
//! whatever it could produce, with problems listed as structured
//! [`Diagnostic`]s. The `write_` functions turn an assembly into an output
//! format, and refuse if it has errors.
//!
//! ```
//! use rasm_one::{assemble, write_bin, MemoryProvider, Options};
//!
//! let mut files = MemoryProvider::new();
//! files.insert("main.s", " nop\n");
//! let assembly = assemble(&Options::new("main.s"), &files);
//! assert_eq!(write_bin(&assembly), Ok(vec![0x4E, 0x71]));
//! ```
//!
//! # Diagnostics
//!
//! Each diagnostic has a stable code and, if it has a location, a [`Span`] for
//! an editor to mark. Related locations include any `include` or macro call
//! that led there.
//!
//! ```
//! use rasm_one::{assemble, MemoryProvider, Options, Severity};
//!
//! let mut files = MemoryProvider::new();
//! files.insert("main.s", " nop\n bogus\n");
//! let assembly = assemble(&Options::new("main.s"), &files);
//!
//! let diagnostic = &assembly.diagnostics[0];
//! assert_eq!(diagnostic.severity, Severity::Error);
//! assert_eq!(diagnostic.code, "unknown-mnemonic");
//! let span = diagnostic.span.expect("this diagnostic has a location");
//! assert_eq!((span.line, span.col, span.len), (2, 2, 5));
//! assert_eq!(
//!     assembly.render_diagnostic(diagnostic),
//!     "main.s:2:2: error[unknown-mnemonic]: unknown mnemonic bogus",
//! );
//! ```
//!
//! # Symbols and the line table
//!
//! [`Assembly::symbols`] lists defined symbols. Every emitted byte, including
//! padding, is covered by [`Assembly::line_table`], which can be read in both
//! directions.
//!
//! ```
//! use rasm_one::{assemble, MemoryProvider, Options};
//!
//! let mut files = MemoryProvider::new();
//! files.insert("main.s", "start: nop\n rts\nsize equ 4\n");
//! let assembly = assemble(&Options::new("main.s"), &files);
//!
//! // A section offset, or a constant if there is no section.
//! let start = &assembly.symbols[0];
//! assert_eq!((start.name.as_str(), start.section, start.value), ("start", Some(0), 0));
//! let size = &assembly.symbols[1];
//! assert_eq!((size.name.as_str(), size.section, size.value), ("size", None, 4));
//!
//! // Address to source: section 0, offset 2.
//! let entry = assembly.line_for_address(0, 2).expect("offset 2 was emitted");
//! assert_eq!(assembly.files[entry.span.file as usize], "main.s");
//! assert_eq!(entry.span.line, 2);
//!
//! // Source to addresses: file 0, line 1.
//! let entries = assembly.addresses_for_line(0, 1);
//! assert_eq!((entries[0].section, entries[0].offset, entries[0].len), (0, 0, 2));
//! ```
//!
//! # Includes and macros
//!
//! Code brought in from elsewhere keeps the location of its own source line.
//! [`LineEntry::expansion`] leads through [`Assembly::expansions`] to the line
//! that brought it in.
//!
//! ```
//! use rasm_one::{assemble, ExpansionKind, MemoryProvider, Options};
//!
//! let mut files = MemoryProvider::new();
//! files.insert("main.s", "twice macro\n nop\n nop\n endm\n\n twice\n");
//! let assembly = assemble(&Options::new("main.s"), &files);
//!
//! let entry = assembly.line_for_address(0, 0).expect("offset 0 was emitted");
//! assert_eq!(entry.span.line, 2);
//! let expansion = assembly.expansions[entry.expansion.expect("came from a macro") as usize];
//! assert_eq!(expansion.kind, ExpansionKind::Macro);
//! assert_eq!(expansion.site.line, 6);
//! ```
//!
//! # Providing files
//!
//! A provider returns [`FileError::NotFound`] for a file that doesn't exist, and
//! the assembler then tries the next place it might be: as written, beside the
//! main source, then in each directory of [`Options::include_dirs`]. Paths are
//! passed with `.` and `dir/..` already removed.
//!
//! ```
//! use rasm_one::{assemble, write_bin, FileError, FileProvider, Options};
//!
//! /// A provider serving one fixed include.
//! struct Project;
//!
//! impl FileProvider for Project {
//!     fn read(&self, path: &str, _including_file: Option<&str>) -> Result<Vec<u8>, FileError> {
//!         match path {
//!             "main.s" => Ok(b" include \"defs.i\"\n dc.b value\n".to_vec()),
//!             "defs.i" => Ok(b"value equ 7\n".to_vec()),
//!             _ => Err(FileError::NotFound(path.to_string())),
//!         }
//!     }
//! }
//!
//! let assembly = assemble(&Options::new("main.s"), &Project);
//! assert_eq!(write_bin(&assembly), Ok(vec![7]));
//! ```

mod atom;
mod error;
mod expr;
mod float;
mod isa;
mod layout;
mod lexer;
mod output;
mod parser;
mod source;
mod types;

pub use error::{Diagnostic, FileError, OutputError, Related, Severity};
pub use output::bin::write_bin;
pub use output::hunk::{write_hunk_exe, write_hunk_exe_with, write_hunk_object, HunkExeOptions};
pub use output::listing::write_listing;
pub use source::{FileId, FileProvider, MemoryProvider, Span};
pub use types::{Cpu, CpuSet, MemKind, SectionKind, Size};

/// Assembly settings. Create these with [`Options::new`], then set fields.
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Options {
    /// Path of the main source, as understood by the [`FileProvider`].
    pub main: String,
    /// Target CPU.
    pub cpu: Cpu,
    /// Whether a floating-point unit is available. By default only the 68040 and
    /// 68060, which have one built in, can use FPU instructions.
    pub fpu: FpuChoice,
    /// Whether a 68851 MMU is available alongside a 68020 or 68030.
    pub mmu_68851: bool,
    /// Directories to search for `include` and `incbin` files.
    pub include_dirs: Vec<String>,
    /// Set this for a raw binary, where labels are final addresses. Displacements
    /// from labels can then use their smallest form on 68020 and later, whereas a
    /// relocatable file needs room for a 32-bit relocation.
    pub fixed_addresses: bool,
    /// The most bytes all sections may hold in total, so that a few characters of
    /// source can't exhaust memory. More is an error.
    pub max_output_bytes: i64,
    /// The most statements that may be generated, which macros and `rept` could
    /// otherwise multiply without bound. More is an error.
    pub max_atoms: usize,
    /// What to optimise. Off by default, like vasm's `-no-opt`.
    pub optimisations: Optimisations,
}

/// Which optimisations apply, numbered like vasm's `-opt-oN` and `opt oN`.
///
/// Only some of vasm's rules exist yet, so this is not yet what vasm does by default.
/// Those implemented: 1, branches; 2, displacements of zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct Optimisations {
    /// False makes this like vasm's `-no-opt`: no rule applies, and `opt` can't change that.
    pub enabled: bool,
    rules: u32,
}

impl Optimisations {
    const IMPLEMENTED: u32 = 1 << RULE_BRANCHES | 1 << RULE_DISPLACEMENTS;

    /// Every rule that exists, which can be changed with `opt`.
    pub fn all() -> Self {
        Self {
            enabled: true,
            rules: Self::IMPLEMENTED,
        }
    }

    /// No rule until `opt` asks for one, like vasm's `-opt-o-`.
    pub fn none() -> Self {
        Self {
            enabled: true,
            rules: 0,
        }
    }

    /// Switches a rule, returning false if it doesn't exist.
    pub fn set_rule(&mut self, rule: u32, on: bool) -> bool {
        let bit = 1u32.checked_shl(rule).unwrap_or(0);
        if bit & Self::IMPLEMENTED == 0 {
            return false;
        }
        self.rules = if on {
            self.rules | bit
        } else {
            self.rules & !bit
        };
        true
    }

    /// Switches every rule that exists.
    pub fn set_all_rules(&mut self, on: bool) {
        self.rules = if on { Self::IMPLEMENTED } else { 0 };
    }

    /// Whether a rule applies.
    pub(crate) fn rule(&self, rule: u32) -> bool {
        self.enabled && self.rules & (1 << rule) != 0
    }
}

/// Rule 1: unsized branches take the smallest form that reaches.
pub(crate) const RULE_BRANCHES: u32 = 1;
/// Rule 2: `0(An)` becomes `(An)`.
pub(crate) const RULE_DISPLACEMENTS: u32 = 2;

impl Options {
    /// Options for assembling `main` for a 68000.
    pub fn new(main: impl Into<String>) -> Self {
        Self {
            main: main.into(),
            cpu: Cpu::default(),
            fpu: FpuChoice::default(),
            mmu_68851: false,
            include_dirs: Vec::new(),
            fixed_addresses: false,
            max_output_bytes: 256 * 1024 * 1024,
            max_atoms: 4_000_000,
            optimisations: Optimisations::default(),
        }
    }
}

/// Whether a floating-point unit is available.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FpuChoice {
    /// Only if the CPU has one built in.
    #[default]
    BuiltIn,
    /// A 68881 or 68882 alongside a 68020 or 68030.
    Coprocessor,
    /// Not even if the CPU has one built in.
    None,
}

/// What a relocation's value means.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RelocKind {
    /// The field gets the target's address added.
    Absolute,
    /// The field gets the distance to the target's address added.
    PcRelative,
}

/// What a relocation refers to.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelocTarget {
    /// A section, by index.
    Section(usize),
    /// A symbol from another object file.
    External(String),
}

/// A field that must be adjusted by the final address of a section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reloc {
    /// Offset of the field in the containing section.
    pub offset: u32,
    /// Width of the field in bytes.
    pub size: u8,
    /// What the field refers to.
    pub target: RelocTarget,
    /// Kind of relocation.
    pub kind: RelocKind,
    /// Source of the field.
    pub span: Span,
}

/// An assembled section.
#[derive(Debug, Clone)]
pub struct Section {
    /// Name.
    pub name: String,
    /// Kind.
    pub kind: SectionKind,
    /// Memory requirement.
    pub mem: MemKind,
    /// Fixed address given by `org`. Its labels are constants.
    pub origin: Option<u32>,
    /// True for an `offset` block, which only defines symbols. Holds no bytes.
    pub dummy: bool,
    /// Contents.
    pub data: Vec<u8>,
    /// Relocations, in the order vasm lists them: by source position, and last
    /// field first within an instruction.
    pub relocs: Vec<Reloc>,
}

/// A defined symbol.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// Name.
    pub name: String,
    /// Containing section, or `None` for a constant.
    pub section: Option<usize>,
    /// Offset in the section, or the constant's value.
    pub value: i64,
    /// Where it is defined.
    pub span: Span,
    /// True for a local label (`.x` or `x$`). Several can share a name.
    pub local: bool,
    /// True if made visible to other object files.
    pub exported: bool,
}

/// What caused an [`Expansion`].
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExpansionKind {
    /// An `include` directive.
    Include,
    /// A macro call.
    Macro,
    /// One iteration of a `rept` block.
    Repeat,
}

impl ExpansionKind {
    pub(crate) fn description(self) -> &'static str {
        match self {
            ExpansionKind::Include => "included from here",
            ExpansionKind::Macro => "expanded from this macro call",
            ExpansionKind::Repeat => "repeated by this rept",
        }
    }
}

/// A place where source was brought in from elsewhere. Following `parent`
/// leads out to the main source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Expansion {
    /// What caused it.
    pub kind: ExpansionKind,
    /// The line that caused it.
    pub site: Span,
    /// The expansion containing that line, if any.
    pub parent: Option<u32>,
}

/// Maps a contiguous run of emitted bytes to the source that produced it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineEntry {
    /// Section index.
    pub section: usize,
    /// Offset of the first byte in the section.
    pub offset: u32,
    /// Number of bytes.
    pub len: u32,
    /// Source position.
    pub span: Span,
    /// Index into [`Assembly::expansions`] of what brought this code in.
    pub expansion: Option<u32>,
}

/// Settings for hunk output that a source chose with `opt`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HunkSettings {
    /// `d-`: write no symbol table.
    pub no_symbols: bool,
    /// `x+`: list only exported symbols in the symbol table.
    pub exported_symbols_only: bool,
    /// `l+` or `l-`: write an object or an executable, whichever was asked for.
    pub linkable: Option<bool>,
}

/// Result of an assembly. Holds whatever could be produced even after errors.
#[derive(Debug, Clone, Default)]
pub struct Assembly {
    /// Names of the source files; a [`FileId`] indexes this list.
    pub files: Vec<String>,
    /// Hunk output settings from `opt`.
    pub hunk_settings: HunkSettings,
    /// Name given by `idnt`, used for the unit of an object file.
    pub unit_name: Option<String>,
    /// Sections in order of first appearance.
    pub sections: Vec<Section>,
    /// Defined symbols.
    pub symbols: Vec<Symbol>,
    /// Address-to-source map, sorted by section and offset.
    pub line_table: Vec<LineEntry>,
    /// Includes and macro expansions that [`LineEntry::expansion`] refers to.
    pub expansions: Vec<Expansion>,
    /// Errors and warnings.
    pub diagnostics: Vec<Diagnostic>,
    /// The limit from [`Options::max_output_bytes`], which also applies to a raw
    /// binary's gaps.
    pub max_output_bytes: i64,
}

impl Assembly {
    /// True if any diagnostic is an error.
    pub fn has_errors(&self) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.severity == Severity::Error)
    }

    /// Finds the source of the byte at `offset` in `section`.
    pub fn line_for_address(&self, section: usize, offset: u32) -> Option<&LineEntry> {
        let index = self
            .line_table
            .partition_point(|e| (e.section, e.offset) <= (section, offset));
        let entry = self.line_table.get(index.checked_sub(1)?)?;
        let inside = entry.section == section && offset - entry.offset < entry.len;
        inside.then_some(entry)
    }

    /// Returns every run of bytes generated by `line` of `file`.
    pub fn addresses_for_line(&self, file: FileId, line: u32) -> Vec<&LineEntry> {
        self.line_table
            .iter()
            .filter(|e| e.span.file == file && e.span.line == line)
            .collect()
    }

    /// Formats a diagnostic as `file:line:col: severity[code]: message`.
    pub fn render_diagnostic(&self, diagnostic: &Diagnostic) -> String {
        let severity = match diagnostic.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Info => "info",
        };
        let body = format!("{severity}[{}]: {}", diagnostic.code, diagnostic.message);
        match diagnostic.span {
            Some(span) => {
                let file = self
                    .files
                    .get(span.file as usize)
                    .map_or("?", String::as_str);
                format!("{file}:{}:{}: {body}", span.line, span.col)
            }
            None => body,
        }
    }
}

/// Assembles `options.main`. Failures are reported through
/// [`Assembly::diagnostics`], never by panicking.
pub fn assemble(options: &Options, provider: &dyn FileProvider) -> Assembly {
    let mut parser = parser::Parser::new(options, provider);
    if let Err(error) = parser.parse_main(&options.main) {
        return Assembly {
            diagnostics: vec![Diagnostic {
                severity: Severity::Error,
                code: "file-read",
                message: error.to_string(),
                span: None,
                related: Vec::new(),
            }],
            ..Assembly::default()
        };
    }
    let built = layout::build(parser.finish(options.max_output_bytes));
    Assembly {
        files: built.files,
        unit_name: built.unit_name,
        hunk_settings: built.hunk_settings,
        max_output_bytes: built.max_output_bytes,
        sections: built.sections,
        symbols: built.symbols,
        line_table: built.line_table,
        expansions: built.expansions,
        diagnostics: built.diagnostics,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(source: &str) -> Assembly {
        let mut files = MemoryProvider::new();
        files.insert("t.s", source);
        assemble(&Options::new("t.s"), &files)
    }

    #[test]
    fn test_assemble_missing_main_file_is_diagnostic() {
        let assembly = assemble(&Options::new("nope.s"), &MemoryProvider::new());
        assert_eq!(assembly.diagnostics[0].code, "file-read");
        assert_eq!(assembly.diagnostics[0].span, None);
    }

    #[test]
    fn test_assemble_continues_after_error_line() {
        let assembly = run(" bogus\n bogus2\n nop\n");
        assert_eq!(assembly.diagnostics.len(), 2);
        assert_eq!(assembly.sections[0].data, [0x4E, 0x71]);
    }

    #[test]
    fn test_assemble_diagnostic_has_line_and_column() {
        let assembly = run("\n  bogus\n");
        let span = assembly.diagnostics[0].span;
        assert_eq!(span.map(|s| (s.line, s.col, s.len)), Some((2, 3, 5)));
        assert_eq!(
            assembly.render_diagnostic(&assembly.diagnostics[0]),
            "t.s:2:3: error[unknown-mnemonic]: unknown mnemonic bogus"
        );
    }

    #[test]
    fn test_assemble_redefined_label_points_at_first_definition() {
        let assembly = run("foo: nop\nfoo: nop\n");
        let diagnostic = &assembly.diagnostics[0];
        assert_eq!(diagnostic.code, "redefined-symbol");
        assert_eq!(diagnostic.related[0].span.line, 1);
    }

    #[test]
    fn test_assemble_undefined_symbol_is_located_at_its_use() {
        let assembly = run(" nop\n dc.l 1+missing\n");
        let diagnostic = &assembly.diagnostics[0];
        assert_eq!(diagnostic.code, "undefined-symbol");
        assert_eq!(diagnostic.span.map(|s| (s.line, s.col)), Some((2, 9)));
    }

    #[test]
    fn test_line_table_covers_every_byte_including_padding() {
        let assembly = run(" dc.b 1\n nop\n blk.w 2,1\n bra.w *\n");
        let data_len = assembly.sections[0].data.len() as u32;
        for offset in 0..data_len {
            assert!(assembly.line_for_address(0, offset).is_some(), "{offset}");
        }
        assert!(assembly.line_for_address(0, data_len).is_none());
        let padding = assembly.line_for_address(0, 1).map(|e| e.span.line);
        assert_eq!(padding, Some(2));
    }

    #[test]
    fn test_addresses_for_line_returns_instruction_bytes() {
        let assembly = run(" nop\n\n rts\n");
        let entries = assembly.addresses_for_line(0, 3);
        assert_eq!(entries.len(), 1);
        assert_eq!((entries[0].offset, entries[0].len), (2, 2));
    }

    #[test]
    fn test_symbols_exported_with_section_offset() {
        let assembly = run(" nop\nfoo: rts\n");
        assert_eq!(assembly.symbols[0].name, "foo");
        assert_eq!(assembly.symbols[0].value, 2);
        assert_eq!(assembly.symbols[0].section, Some(0));
    }

    #[test]
    fn test_relocs_narrow_fields_cover_only_their_bytes() {
        let assembly = run(" move.b #f,d0\n move.w f(a0,d0.w),d0\nf:\n");
        let relocs = &assembly.sections[0].relocs;
        assert_eq!((relocs[0].offset, relocs[0].size), (3, 1));
        assert_eq!((relocs[1].offset, relocs[1].size), (7, 1));
    }

    #[test]
    fn test_relocs_recorded_for_section_addresses() {
        let assembly = run(" move.l #foo,d0\nfoo: dc.l foo\n");
        let relocs = &assembly.sections[0].relocs;
        assert_eq!(relocs.len(), 2);
        assert_eq!((relocs[0].offset, relocs[0].size), (2, 4));
        assert_eq!((relocs[1].offset, relocs[1].size), (6, 4));
    }

    #[test]
    fn test_write_bin_overlapping_sections_is_error() {
        let assembly = run(" section a,code\n nop\n section b,data\n dc.w 1\n");
        assert_eq!(
            write_bin(&assembly),
            Err(OutputError::Overlap("a".to_string(), "b".to_string()))
        );
    }

    #[test]
    fn test_write_bin_refuses_failed_assembly() {
        assert_eq!(write_bin(&run(" bogus\n")), Err(OutputError::HasErrors));
    }

    #[test]
    fn test_write_hunk_exe_org_is_error() {
        let assembly = run(" org $1000\n nop\n");
        assert_eq!(write_hunk_exe(&assembly), Err(OutputError::FixedAddress));
    }

    #[test]
    fn test_line_table_is_sorted_when_sections_alternate() {
        let source = " section a,code\n nop\n section b,data\n dc.w 1\n section a,code\n rts\n";
        let assembly = run(source);
        let order: Vec<_> = assembly
            .line_table
            .iter()
            .map(|e| (e.section, e.offset, e.span.line))
            .collect();
        assert_eq!(order, [(0, 0, 2), (0, 2, 6), (1, 0, 4)]);
    }

    #[test]
    fn test_line_table_covers_padding_directives() {
        let assembly = run(" dc.b 1\n cnop 0,4\n dc.b 2\n");
        let line = |offset| assembly.line_for_address(0, offset).map(|e| e.span.line);
        assert_eq!(
            (line(0), line(1), line(3), line(4)),
            (Some(1), Some(2), Some(2), Some(3))
        );
    }

    #[test]
    fn test_end_stops_assembly() {
        let assembly = run(" nop\n end\n bogus\n");
        assert!(!assembly.has_errors());
        assert_eq!(assembly.sections[0].data.len(), 2);
    }

    #[test]
    fn test_fail_reports_its_message() {
        let assembly = run(" fail \"oops\"\n");
        assert_eq!(assembly.diagnostics[0].code, "fail");
        assert!(assembly.diagnostics[0].message.contains("oops"));
    }

    fn run_with(files: &[(&str, &str)]) -> Assembly {
        let mut provider = MemoryProvider::new();
        for (path, contents) in files {
            provider.insert(*path, *contents);
        }
        assemble(&Options::new("t.s"), &provider)
    }

    #[test]
    fn test_include_error_reports_file_and_include_chain() {
        let assembly = run_with(&[
            ("t.s", " nop\n include \"a.i\"\n"),
            ("a.i", " include \"b.i\"\n"),
            ("b.i", "\n bogus\n"),
        ]);
        let diagnostic = &assembly.diagnostics[0];
        let span = diagnostic.span.map(|s| (s.file, s.line));
        assert_eq!(span, Some((2, 2)));
        assert_eq!(assembly.files[2], "b.i");
        let chain: Vec<_> = diagnostic
            .related
            .iter()
            .map(|r| (r.span.file, r.span.line))
            .collect();
        assert_eq!(chain, [(1, 1), (0, 2)]);
    }

    #[test]
    fn test_include_emit_error_has_include_chain() {
        let assembly = run_with(&[("t.s", " include \"a.i\"\n"), ("a.i", " dc.l nope\n")]);
        assert_eq!(assembly.diagnostics[0].code, "undefined-symbol");
        assert_eq!(assembly.diagnostics[0].related[0].span.line, 1);
    }

    #[test]
    fn test_include_cycle_is_error() {
        let assembly = run_with(&[("t.s", " include \"a.i\"\n"), ("a.i", " include \"t.s\"\n")]);
        assert_eq!(assembly.diagnostics[0].code, "include-cycle");
    }

    #[test]
    fn test_include_missing_file_is_located_at_directive() {
        let assembly = run_with(&[("t.s", " nop\n include \"nope.i\"\n rts\n")]);
        assert_eq!(assembly.diagnostics[0].code, "file-read");
        assert_eq!(assembly.diagnostics[0].span.map(|s| s.line), Some(2));
        assert_eq!(assembly.sections[0].data.len(), 4);
    }

    #[test]
    fn test_line_table_maps_included_code_to_its_file_and_expansion() {
        let assembly = run_with(&[("t.s", " include \"a.i\"\n rts\n"), ("a.i", " nop\n")]);
        let included = assembly.line_for_address(0, 0);
        assert_eq!(included.map(|e| (e.span.file, e.span.line)), Some((1, 1)));
        let expansion = included.and_then(|e| e.expansion);
        assert_eq!(expansion, Some(0));
        assert_eq!(assembly.expansions[0].site.line, 1);
        let main = assembly.line_for_address(0, 2);
        assert_eq!(main.and_then(|e| e.expansion), None);
    }

    /// A provider failure other than a missing file must not be hidden.
    #[test]
    fn test_include_provider_failure_is_reported() {
        struct Failing;
        impl FileProvider for Failing {
            fn read(&self, path: &str, from: Option<&str>) -> Result<Vec<u8>, FileError> {
                match (path, from) {
                    ("t.s", None) => Ok(b" include \"a.i\"\n".to_vec()),
                    _ => Err(FileError::Other("disk on fire".to_string())),
                }
            }
        }
        let assembly = assemble(&Options::new("t.s"), &Failing);
        assert!(assembly.diagnostics[0].message.contains("disk on fire"));
    }

    #[test]
    fn test_macro_error_reports_body_line_and_call() {
        let assembly = run("m macro\n bogus\n endm\n\n m\n");
        let diagnostic = &assembly.diagnostics[0];
        assert_eq!(diagnostic.span.map(|s| s.line), Some(2));
        assert_eq!(diagnostic.related[0].span.line, 5);
    }

    #[test]
    fn test_line_table_maps_macro_bytes_to_body_and_call() {
        let assembly = run("m macro\n nop\n endm\n m\n");
        let entry = assembly.line_for_address(0, 0);
        assert_eq!(entry.map(|e| e.span.line), Some(2));
        let expansion = entry.and_then(|e| e.expansion).map(|id| id as usize);
        assert_eq!(
            expansion.map(|id| assembly.expansions[id].site.line),
            Some(4)
        );
        assert_eq!(
            expansion.map(|id| assembly.expansions[id].kind),
            Some(ExpansionKind::Macro)
        );
    }

    #[test]
    fn test_macro_recursion_is_error_not_crash() {
        let assembly = run("m macro\n m\n endm\n m\n");
        assert!(assembly.diagnostics.iter().any(|d| d.code == "macro-depth"));
    }

    #[test]
    fn test_unterminated_blocks_are_located() {
        let assembly = run("\n if 1\n");
        assert_eq!(assembly.diagnostics[0].span.map(|s| s.line), Some(2));
        let assembly = run("\n rept 2\n nop\n");
        assert_eq!(assembly.diagnostics[0].span.map(|s| s.line), Some(2));
        let assembly = run("\nm macro\n nop\n");
        assert_eq!(assembly.diagnostics[0].span.map(|s| s.line), Some(2));
    }

    #[test]
    fn test_local_label_symbol_keeps_its_written_name() {
        let assembly = run("a: nop\n.l: nop\n");
        assert_eq!(assembly.symbols[1].name, ".l");
    }

    #[test]
    fn test_symbols_leave_out_internal_counters() {
        let assembly = run("m macro\n endm\n m\n rept 1\n endr\nx rs.w 1\nl: nop\n");
        let names: Vec<_> = assembly.symbols.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["x", "l"]);
    }

    #[test]
    fn test_printt_reports_text_as_info_not_error() {
        let assembly = run(" printt \"hello\"\n printv 2+3\n nop\n");
        assert_eq!(assembly.diagnostics[0].severity, Severity::Info);
        assert_eq!(assembly.diagnostics[0].message, "hello");
        assert!(assembly.diagnostics[1].message.starts_with("5 "));
        assert_eq!(write_bin(&assembly), Ok(vec![0x4E, 0x71]));
    }

    #[test]
    fn test_offset_block_has_no_bytes_or_lines() {
        let assembly = run(" offset\nx ds.w 2\ny ds.b 0\n");
        assert!(assembly.line_table.is_empty());
        assert_eq!(assembly.symbols[1].value, 4);
        assert_eq!(assembly.symbols[1].section, None);
    }

    #[test]
    fn test_assemble_zpc_displacement_is_not_pc_relative() {
        let mut files = MemoryProvider::new();
        files.insert("t.s", " move.w (4,zpc),d0\n");
        let mut options = Options::new("t.s");
        options.cpu = Cpu::M68020;
        let assembly = assemble(&options, &files);
        assert_eq!(
            assembly.sections[0].data,
            [0x30, 0x3B, 0x01, 0xE0, 0x00, 0x04]
        );
    }

    #[test]
    fn test_encode_grown_modes_use_the_full_format_mode_bits() {
        let mut files = MemoryProvider::new();
        files.insert("t.s", " divs (100000,a6),d4\n jmp (100000,pc)\n");
        let mut options = Options::new("t.s");
        options.cpu = Cpu::M68020;
        let assembly = assemble(&options, &files);
        assert_eq!(
            assembly.sections[0].data,
            [
                0x89, 0xF6, 0x01, 0x70, 0x00, 0x01, 0x86, 0xA0, // divs
                // In a relocatable section a constant is the displacement itself.
                0x4E, 0xFB, 0x01, 0x70, 0x00, 0x01, 0x86, 0xA0, // jmp
            ]
        );
    }

    fn run_on(source: &str, cpu: Cpu, fixed: bool) -> Assembly {
        let mut files = MemoryProvider::new();
        files.insert("t.s", source);
        let mut options = Options::new("t.s");
        options.cpu = cpu;
        options.fixed_addresses = fixed;
        assemble(&options, &files)
    }

    #[test]
    fn test_unresolved_float_immediates_are_errors_not_panics() {
        for source in [
            " fmove.d #foo,fp0\n",
            " fmove.x #foo,fp0\n",
            " fmove.p #1/0,fp0\n",
        ] {
            assert!(run_on(source, Cpu::M68040, false).has_errors(), "{source}");
        }
    }

    #[test]
    fn test_deep_parentheses_are_an_error_not_a_crash() {
        let source = format!(" dc.w {}1{}\n", "(".repeat(5000), ")".repeat(5000));
        assert!(run(&source).has_errors());
    }

    #[test]
    fn test_reasonable_parentheses_are_accepted() {
        let source = format!(" dc.w {}1{}\n", "(".repeat(30), ")".repeat(30));
        assert!(!run(&source).has_errors());
    }

    #[test]
    fn test_deep_unary_operators_are_an_error_not_a_crash() {
        assert!(run(&format!(" dc.w {}1\n", "-".repeat(5000))).has_errors());
    }

    #[test]
    fn test_long_operator_chains_are_an_error_not_a_crash() {
        assert!(run(&format!(" dc.w 1{}\n", "+1".repeat(20000))).has_errors());
    }

    #[test]
    fn test_deep_rept_is_an_error_not_a_crash() {
        let source = format!("{}{}", " rept 1\n".repeat(5000), " endr\n".repeat(5000));
        assert!(run(&source).has_errors());
    }

    #[test]
    fn test_limits_stop_runaway_sources() {
        let sources = [
            " ds.b $7fffffff\n",
            " cnop $7fffffff,2\n ds.b $7fffffff\n",
            " rept $7fffffffffff\n endr\n",
            "m macro\n m\n m\n endm\n m\n",
            " rept 100000\n rept 100000\n nop\n endr\n endr\n",
            " offset\n ds.b $7fffffff\n",
        ];
        for source in sources {
            let mut files = MemoryProvider::new();
            files.insert("t.s", source);
            let mut options = Options::new("t.s");
            // The default limit is much higher, which would only make this slower.
            options.max_atoms = 50_000;
            options.max_output_bytes = 1024 * 1024;
            let assembly = assemble(&options, &files);
            assert!(
                assembly
                    .diagnostics
                    .iter()
                    .any(|d| d.code == "limit-exceeded"),
                "{source}"
            );
        }
        for source in [" cnop $ffffffffffff,2\n", " cnop 0,$7fffffffffffffff\n"] {
            assert!(run(source).has_errors(), "{source}");
        }
    }

    #[test]
    fn test_write_bin_gap_beyond_limit_is_error() {
        let assembly = run(" org 0\n nop\n org $ffffffff\n nop\n");
        assert_eq!(write_bin(&assembly), Err(OutputError::TooLarge));
    }

    #[test]
    fn test_write_bin_empty_org_inside_a_section_is_overlap() {
        let assembly = run(" org $1000\n dc.l 1,2,3,4\n org $1008\n");
        assert!(matches!(
            write_bin(&assembly),
            Err(OutputError::Overlap(..))
        ));
    }

    #[test]
    fn test_extreme_addresses_do_not_overflow() {
        for source in [
            " offset $7fffffffffffffff\n ds.b 2\nx:\n",
            " nop\n rorg -$7fffffffffffffff-1\n",
            " nop\n bra -$7fffffffffffffff-1\n",
            " nop\n lea (-$7fffffffffffffff-1)(pc),a0\n",
        ] {
            for fixed in [false, true] {
                run_on(source, Cpu::M68020, fixed);
            }
        }
    }

    #[test]
    fn test_values_over_32_bits_are_errors() {
        for source in [
            " jmp $123456789\n",
            " bra.l $123456789\n",
            " move.l (100000000000,a0),d0\n",
        ] {
            assert!(run_on(source, Cpu::M68020, true).has_errors(), "{source}");
        }
    }

    #[test]
    fn test_long_equate_chain_settles() {
        let mut source = String::from(" dc.w s0\n");
        for link in 0..40 {
            source.push_str(&format!("s{link} equ s{}\n", link + 1));
        }
        source.push_str("s40 equ 1\n");
        let assembly = run(&source);
        assert!(!assembly.has_errors(), "{:?}", assembly.diagnostics.first());
    }

    #[test]
    fn test_macro_substituted_lines_report_the_line_not_a_false_column() {
        let source = "m macro\n move.l \\1,zz\n endm\n m d0_very_long_argument_replacement\n";
        let assembly = run(source);
        let span = assembly.diagnostics[0].span;
        assert_eq!(span.map(|s| (s.line, s.col)), Some((2, 1)));
    }

    #[test]
    fn test_macro_lines_without_parameters_keep_exact_columns() {
        let assembly = run("m macro\n bogus\n endm\n m\n");
        let span = assembly.diagnostics[0].span;
        assert_eq!(span.map(|s| (s.line, s.col)), Some((2, 2)));
    }

    #[test]
    fn test_opt_warns_for_each_ignored_option() {
        let assembly = run(" opt c-,o+\n nop\n");
        let messages: Vec<_> = assembly
            .diagnostics
            .iter()
            .map(|d| d.message.as_str())
            .collect();
        assert_eq!(messages, ["option c- is ignored", "option o+ is ignored"]);
        assert_eq!(assembly.diagnostics[0].severity, Severity::Warning);
        assert_eq!(write_bin(&assembly), Ok(vec![0x4E, 0x71]));
    }

    #[test]
    fn test_opt_p_changes_the_processor() {
        let assembly = run(" opt p=68020/68881\n bfextu d0{1:2},d1\n fnop\n");
        assert!(
            assembly.diagnostics.is_empty(),
            "{:?}",
            assembly.diagnostics
        );
    }

    #[test]
    fn test_ds_negative_count_is_error() {
        assert!(run(" ds.b -1\n").has_errors());
    }
}
