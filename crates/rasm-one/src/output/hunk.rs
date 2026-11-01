//! Amiga hunk executable and object output.

use crate::error::OutputError;
use crate::types::{MemKind, SectionKind};
use crate::{Assembly, Reloc, RelocKind, RelocTarget, Section, Symbol, Target};

const HUNK_UNIT: u32 = 0x3E7;
const HUNK_NAME: u32 = 0x3E8;
const HUNK_CODE: u32 = 0x3E9;
const HUNK_DATA: u32 = 0x3EA;
const HUNK_BSS: u32 = 0x3EB;
const HUNK_RELOC32: u32 = 0x3EC;
const HUNK_RELOC16: u32 = 0x3ED;
const HUNK_RELOC8: u32 = 0x3EE;
const HUNK_EXT: u32 = 0x3EF;
const HUNK_SYMBOL: u32 = 0x3F0;
const HUNK_END: u32 = 0x3F2;
const HUNK_HEADER: u32 = 0x3F3;
const HUNK_RELOC32SHORT: u32 = 0x3F7;
const HUNK_DREL16: u32 = 0x3F8;
const HUNK_RELRELOC32: u32 = 0x3FD;

const EXT_DEF: u8 = 0x01;
const EXT_ABS: u8 = 0x02;
const EXT_REF32: u8 = 0x81;
const EXT_REF16: u8 = 0x83;
const EXT_REF8: u8 = 0x84;
const EXT_RELREF32: u8 = 0x88;
const EXT_ABSREF16: u8 = 0x8A;
const EXT_ABSREF8: u8 = 0x8B;

const MEM_CHIP: u32 = 0x4000_0000;
const MEM_FAST: u32 = 0x8000_0000;

/// Largest section offset the short relocation format can hold.
const MAX_SHORT_OFFSET: u32 = 0xFFFF;

/// Name of the hunk used if only constants are exported.
const DEFAULT_HUNK_NAME: &str = "CODE";

/// The sections that become hunks, and the hunk number of each section.
struct Hunks<'a> {
    list: Vec<(usize, &'a Section)>,
    numbers: Vec<Option<u32>>,
}

/// Sections become hunks in order of first appearance, leaving out those with
/// neither bytes nor symbols. Any symbol counts, even a local label that won't
/// be listed.
fn select_hunks(assembly: &Assembly) -> Hunks<'_> {
    let has_symbol = |index: usize| assembly.symbols.iter().any(|s| s.section == Some(index));
    let mut hunks = Hunks {
        list: Vec::new(),
        numbers: vec![None; assembly.sections.len()],
    };
    for (index, section) in assembly.sections.iter().enumerate() {
        if !section.dummy && (!section.data.is_empty() || has_symbol(index)) {
            hunks.numbers[index] = Some(hunks.list.len() as u32);
            hunks.list.push((index, section));
        }
    }
    hunks
}

fn check_writable(assembly: &Assembly) -> Result<(), OutputError> {
    if assembly.has_errors() {
        return Err(OutputError::HasErrors);
    }
    if assembly
        .sections
        .iter()
        .any(|s| !s.dummy && s.origin.is_some())
    {
        return Err(OutputError::FixedAddress);
    }
    Ok(())
}

/// Settings for [`write_hunk_exe_with`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HunkExeOptions {
    /// Use only hunk types that Kickstart 1.x can load. Its loader refuses the
    /// compact relocation format with error 121, "file is not executable".
    pub kickstart1: bool,
}

/// Writes an executable as vasm does by default, which needs Kickstart 2.0 or
/// later. See [`write_hunk_exe_with`] for one that also loads on 1.x.
pub fn write_hunk_exe(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    write_hunk_exe_with(assembly, HunkExeOptions::default())
}

/// Writes an executable.
pub fn write_hunk_exe_with(
    assembly: &Assembly,
    options: HunkExeOptions,
) -> Result<Vec<u8>, OutputError> {
    if assembly.target != Target::Relocatable {
        return Err(OutputError::WrongTarget);
    }
    check_writable(assembly)?;
    // The source can ask for the other kind of file with `opt l+`.
    if assembly.hunk_settings.linkable == Some(true) {
        return object_bytes(assembly);
    }
    executable_bytes(assembly, options)
}

fn executable_bytes(assembly: &Assembly, options: HunkExeOptions) -> Result<Vec<u8>, OutputError> {
    let hunks = select_hunks(assembly);
    let mut out = Writer::default();
    out.long(HUNK_HEADER);
    out.long(0); // No resident library names.
    if hunks.list.is_empty() {
        // An executable needs at least one hunk, even if it is empty.
        for field in [1, 0, 0, 0, HUNK_CODE, 0, HUNK_END] {
            out.long(field);
        }
        return Ok(out.bytes);
    }
    out.long(hunks.list.len() as u32);
    out.long(0);
    out.long(hunks.list.len() as u32 - 1);
    for (_, section) in &hunks.list {
        out.long(size_in_longs(section) | mem_flags(section.mem));
    }
    for &(index, section) in &hunks.list {
        write_contents(section, 0, &mut out);
        if section.kind != SectionKind::Bss {
            write_exe_relocs(section, &hunks.numbers, options, &mut out)?;
        }
        write_symbols(assembly, index, &mut out);
        out.long(HUNK_END);
    }
    Ok(out.bytes)
}

/// Writes a linkable object file.
pub fn write_hunk_object(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    if assembly.target != Target::Relocatable {
        return Err(OutputError::WrongTarget);
    }
    check_writable(assembly)?;
    // The source can ask for the other kind of file with `opt l-`.
    if assembly.hunk_settings.linkable == Some(false) {
        return executable_bytes(assembly, HunkExeOptions::default());
    }
    object_bytes(assembly)
}

fn object_bytes(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    let hunks = select_hunks(assembly);
    let mut out = Writer::default();
    out.long(HUNK_UNIT);
    let main = assembly.files.first().map_or("", String::as_str);
    let unit = match &assembly.unit_name {
        Some(name) => name.as_str(),
        None => main.rsplit('/').next().unwrap_or(main),
    };
    out.name(unit, 0);

    let is_constant_export = |s: &&Symbol| s.exported && s.section.is_none();
    if hunks.list.is_empty() {
        // Constants need a hunk to be exported from; otherwise leave it unnamed.
        if assembly.symbols.iter().any(|s| is_constant_export(&s)) {
            out.long(HUNK_NAME);
            out.name(DEFAULT_HUNK_NAME, 0);
        }
        out.long(HUNK_CODE);
        out.long(0);
        write_external_info(assembly, None, true, &[], &mut out);
        out.long(HUNK_END);
        return Ok(out.bytes);
    }
    for (position, &(index, section)) in hunks.list.iter().enumerate() {
        out.long(HUNK_NAME);
        out.name(&section.name, 0);
        write_contents(section, mem_flags(section.mem), &mut out);
        let relocs: &[Reloc] = if section.kind == SectionKind::Bss {
            &[]
        } else {
            &section.relocs
        };
        write_object_relocs(section, relocs, &hunks.numbers, &mut out)?;
        write_external_info(assembly, Some(index), position == 0, relocs, &mut out);
        write_symbols(assembly, index, &mut out);
        out.long(HUNK_END);
    }
    Ok(out.bytes)
}

#[derive(Default)]
struct Writer {
    bytes: Vec<u8>,
}

impl Writer {
    fn long(&mut self, value: u32) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    fn word(&mut self, value: u16) {
        self.bytes.extend_from_slice(&value.to_be_bytes());
    }

    /// Writes a length in longs, with `kind` in the top byte, then the
    /// zero-padded name.
    fn name(&mut self, name: &str, kind: u8) {
        let bytes = name.as_bytes();
        self.long(u32::from(kind) << 24 | (bytes.len() as u32).div_ceil(4));
        self.bytes.extend_from_slice(bytes);
        self.zeros((4 - bytes.len() % 4) % 4);
    }

    fn zeros(&mut self, count: usize) {
        self.bytes.resize(self.bytes.len() + count, 0);
    }
}

fn size_in_longs(section: &Section) -> u32 {
    (section.data.len() as u32).div_ceil(4)
}

fn mem_flags(mem: MemKind) -> u32 {
    match mem {
        MemKind::Any => 0,
        MemKind::Chip => MEM_CHIP,
        MemKind::Fast => MEM_FAST,
    }
}

fn write_contents(section: &Section, type_flags: u32, out: &mut Writer) {
    let kind = match section.kind {
        SectionKind::Code => HUNK_CODE,
        SectionKind::Data => HUNK_DATA,
        SectionKind::Bss => HUNK_BSS,
    };
    out.long(kind | type_flags);
    out.long(size_in_longs(section));
    if section.kind == SectionKind::Bss {
        return;
    }
    out.bytes.extend_from_slice(&section.data);
    let padding = (4 - section.data.len() % 4) % 4;
    if section.kind == SectionKind::Code && padding == 2 {
        out.word(0x4E71); // NOP
    } else {
        out.zeros(padding);
    }
}

fn unsupported(section: &Section, reloc: &Reloc) -> OutputError {
    OutputError::UnsupportedReloc {
        section: section.name.clone(),
        offset: reloc.offset,
        size: reloc.size,
        span: reloc.span,
    }
}

/// Offsets grouped by target hunk, in hunk order.
type OffsetsByHunk = Vec<(u32, Vec<u32>)>;

fn add_offset(groups: &mut OffsetsByHunk, hunk: u32, offset: u32) {
    match groups.iter_mut().find(|group| group.0 == hunk) {
        Some(group) => group.1.push(offset),
        None => {
            groups.push((hunk, vec![offset]));
            groups.sort_by_key(|group| group.0);
        }
    }
}

/// The hunk a relocation's section target is in. Only sections with bytes or
/// symbols can be targets, and those always have hunks.
fn target_hunk(
    section: &Section,
    reloc: &Reloc,
    target: usize,
    numbers: &[Option<u32>],
) -> Result<u32, OutputError> {
    numbers
        .get(target)
        .copied()
        .flatten()
        .ok_or_else(|| unsupported(section, reloc))
}

/// Writes offsets up to 64K in the compact format, unless told not to, and any
/// others in the long format.
fn write_exe_relocs(
    section: &Section,
    numbers: &[Option<u32>],
    options: HunkExeOptions,
    out: &mut Writer,
) -> Result<(), OutputError> {
    let mut short = OffsetsByHunk::new();
    let mut long = OffsetsByHunk::new();
    let mut relative = OffsetsByHunk::new();
    for reloc in &section.relocs {
        let target = match &reloc.target {
            RelocTarget::Section(target) => *target,
            RelocTarget::External(name) => {
                return Err(OutputError::UnresolvedExternal {
                    name: name.clone(),
                    span: reloc.span,
                });
            }
        };
        let hunk = target_hunk(section, reloc, target, numbers)?;
        if reloc.size == 4 && reloc.kind == RelocKind::PcRelative {
            // This type has only the compact form, and Kickstart 1.x lacks it.
            if options.kickstart1 || reloc.offset > MAX_SHORT_OFFSET {
                return Err(unsupported(section, reloc));
            }
            add_offset(&mut relative, hunk, reloc.offset);
            continue;
        }
        if reloc.size != 4 {
            return Err(unsupported(section, reloc));
        }
        if reloc.offset <= MAX_SHORT_OFFSET && !options.kickstart1 {
            add_offset(&mut short, hunk, reloc.offset);
        } else {
            add_offset(&mut long, hunk, reloc.offset);
        }
    }
    write_short_relocs(HUNK_RELOC32SHORT, &short, out);
    write_long_relocs(HUNK_RELOC32, &long, out);
    write_short_relocs(HUNK_RELRELOC32, &relative, out);
    Ok(())
}

fn write_short_relocs(hunk_type: u32, groups: &OffsetsByHunk, out: &mut Writer) {
    if groups.is_empty() {
        return;
    }
    out.long(hunk_type);
    let mut words = 0;
    for (hunk, offsets) in groups {
        out.word(offsets.len() as u16);
        out.word(*hunk as u16);
        for &offset in offsets {
            out.word(offset as u16);
        }
        words += 2 + offsets.len();
    }
    out.word(0);
    if (words + 1) % 2 != 0 {
        out.word(0); // Keep the stream long-aligned.
    }
}

fn write_long_relocs(hunk_type: u32, groups: &OffsetsByHunk, out: &mut Writer) {
    if groups.is_empty() {
        return;
    }
    out.long(hunk_type);
    for (hunk, offsets) in groups {
        out.long(offsets.len() as u32);
        out.long(*hunk);
        for &offset in offsets {
            out.long(offset);
        }
    }
    out.long(0);
}

fn write_object_relocs(
    section: &Section,
    relocs: &[Reloc],
    numbers: &[Option<u32>],
    out: &mut Writer,
) -> Result<(), OutputError> {
    let mut absolute = OffsetsByHunk::new();
    let mut pc16 = OffsetsByHunk::new();
    let mut pc8 = OffsetsByHunk::new();
    let mut pc32 = OffsetsByHunk::new();
    let mut base16 = OffsetsByHunk::new();
    for reloc in relocs {
        let RelocTarget::Section(target) = reloc.target else {
            continue; // External references are written separately.
        };
        let hunk = target_hunk(section, reloc, target, numbers)?;
        match (reloc.kind, reloc.size) {
            (RelocKind::Absolute, 4) => add_offset(&mut absolute, hunk, reloc.offset),
            (RelocKind::PcRelative, 4) => add_offset(&mut pc32, hunk, reloc.offset),
            (RelocKind::PcRelative, 2) => add_offset(&mut pc16, hunk, reloc.offset),
            (RelocKind::PcRelative, 1) => add_offset(&mut pc8, hunk, reloc.offset),
            (RelocKind::BaseRelative, 2) => add_offset(&mut base16, hunk, reloc.offset),
            _ => return Err(unsupported(section, reloc)),
        }
    }
    write_long_relocs(HUNK_RELOC32, &absolute, out);
    write_long_relocs(HUNK_RELOC16, &pc16, out);
    write_long_relocs(HUNK_RELOC8, &pc8, out);
    write_long_relocs(HUNK_RELRELOC32, &pc32, out);
    write_long_relocs(HUNK_DREL16, &base16, out);
    Ok(())
}

/// The kind of external reference a relocation needs.
fn external_ref_kind(reloc: &Reloc) -> Option<u8> {
    match (reloc.kind, reloc.size) {
        (RelocKind::Absolute, 4) => Some(EXT_REF32),
        (RelocKind::Absolute, 2) => Some(EXT_ABSREF16),
        (RelocKind::Absolute, 1) => Some(EXT_ABSREF8),
        (RelocKind::PcRelative, 4) => Some(EXT_RELREF32),
        (RelocKind::PcRelative, 2) => Some(EXT_REF16),
        (RelocKind::PcRelative, 1) => Some(EXT_REF8),
        _ => None,
    }
}

/// Writes references to other objects, grouped by symbol and kind in order of
/// first use, then the symbols this hunk exports. Constants are exported from
/// the first hunk.
fn write_external_info(
    assembly: &Assembly,
    section: Option<usize>,
    is_first: bool,
    relocs: &[Reloc],
    out: &mut Writer,
) {
    let mut references: Vec<(&str, u8, Vec<u32>)> = Vec::new();
    for reloc in relocs {
        let RelocTarget::External(name) = &reloc.target else {
            continue;
        };
        let Some(kind) = external_ref_kind(reloc) else {
            continue;
        };
        let existing = references
            .iter_mut()
            .find(|entry| entry.0 == name && entry.1 == kind);
        match existing {
            Some(entry) => entry.2.push(reloc.offset),
            None => references.push((name, kind, vec![reloc.offset])),
        }
    }
    let definitions: Vec<(&Symbol, u8)> = assembly
        .symbols
        .iter()
        .filter(|symbol| symbol.exported && !symbol.local)
        .filter_map(|symbol| match symbol.section {
            None if is_first => Some((symbol, EXT_ABS)),
            Some(_) if symbol.section == section => Some((symbol, EXT_DEF)),
            _ => None,
        })
        .collect();
    if references.is_empty() && definitions.is_empty() {
        return;
    }
    out.long(HUNK_EXT);
    for (name, kind, offsets) in references {
        out.name(name, kind);
        out.long(offsets.len() as u32);
        for offset in offsets {
            out.long(offset);
        }
    }
    for (symbol, kind) in definitions {
        out.name(&symbol.name, kind);
        out.long(symbol.value as u32);
    }
    out.long(0);
}

fn write_symbols(assembly: &Assembly, section: usize, out: &mut Writer) {
    let settings = assembly.hunk_settings;
    if settings.no_symbols {
        return;
    }
    let mut symbols = assembly
        .symbols
        .iter()
        .filter(|symbol| !symbol.local && symbol.section == Some(section))
        .filter(|symbol| symbol.exported || !settings.exported_symbols_only)
        .peekable();
    if symbols.peek().is_none() {
        return;
    }
    out.long(HUNK_SYMBOL);
    for symbol in symbols {
        out.name(&symbol.name, 0);
        out.long(symbol.value as u32);
    }
    out.long(0);
}
