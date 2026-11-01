//! Amiga hunk executable output.

use crate::error::OutputError;
use crate::types::{MemKind, SectionKind};
use crate::{Assembly, Reloc, Section};

const HUNK_CODE: u32 = 0x3E9;
const HUNK_DATA: u32 = 0x3EA;
const HUNK_BSS: u32 = 0x3EB;
const HUNK_RELOC32: u32 = 0x3EC;
const HUNK_SYMBOL: u32 = 0x3F0;
const HUNK_END: u32 = 0x3F2;
const HUNK_HEADER: u32 = 0x3F3;
const HUNK_RELOC32SHORT: u32 = 0x3F7;

const MEM_CHIP: u32 = 0x4000_0000;
const MEM_FAST: u32 = 0x8000_0000;

/// Largest section offset the short relocation format can hold.
const MAX_SHORT_OFFSET: u32 = 0xFFFF;

/// Writes an executable. Sections become hunks in order of first appearance;
/// sections with neither bytes nor symbols are left out.
pub fn write_hunk_exe(assembly: &Assembly) -> Result<Vec<u8>, OutputError> {
    if assembly.has_errors() {
        return Err(OutputError::HasErrors);
    }
    if assembly.sections.iter().any(|s| s.origin.is_some()) {
        return Err(OutputError::FixedAddress);
    }
    let has_symbol = |index: usize| assembly.symbols.iter().any(|s| s.section == Some(index));
    // Maps a section index to its hunk number.
    let mut hunk_numbers = vec![None; assembly.sections.len()];
    let mut hunks = Vec::new();
    for (index, section) in assembly.sections.iter().enumerate() {
        if !section.data.is_empty() || has_symbol(index) {
            hunk_numbers[index] = Some(hunks.len() as u32);
            hunks.push((index, section));
        }
    }

    let mut out = Writer::default();
    out.long(HUNK_HEADER);
    out.long(0); // No resident library names.
    if hunks.is_empty() {
        // An executable needs at least one hunk, even if it is empty.
        for field in [1, 0, 0, 0, HUNK_CODE, 0, HUNK_END] {
            out.long(field);
        }
        return Ok(out.bytes);
    }
    out.long(hunks.len() as u32);
    out.long(0);
    out.long(hunks.len() as u32 - 1);
    for (_, section) in &hunks {
        out.long(size_in_longs(section) | mem_flags(section.mem));
    }
    for &(index, section) in &hunks {
        write_contents(section, &mut out);
        if section.kind != SectionKind::Bss {
            write_relocs(section, &hunk_numbers, &mut out)?;
        }
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

fn write_contents(section: &Section, out: &mut Writer) {
    let kind = match section.kind {
        SectionKind::Code => HUNK_CODE,
        SectionKind::Data => HUNK_DATA,
        SectionKind::Bss => HUNK_BSS,
    };
    out.long(kind);
    out.long(size_in_longs(section));
    if section.kind == SectionKind::Bss {
        return;
    }
    out.bytes.extend_from_slice(&section.data);
    let padding = (4 - section.data.len() % 4) % 4;
    if section.kind == SectionKind::Code && padding == 2 {
        out.word(0x4E71); // NOP
    } else {
        out.bytes.extend(std::iter::repeat(0).take(padding));
    }
}

/// Writes offsets up to 64K in the compact format and any others in the long
/// format. Both group offsets by target hunk.
fn write_relocs(
    section: &Section,
    hunk_numbers: &[Option<u32>],
    out: &mut Writer,
) -> Result<(), OutputError> {
    // One (short offsets, long offsets) pair per hunk that is the target.
    let mut by_target: Vec<(u32, Vec<u32>, Vec<u32>)> = Vec::new();
    for reloc in &section.relocs {
        if reloc.size != 4 {
            return Err(unsupported(section, reloc));
        }
        // Only sections with bytes or symbols can be targets, and those have hunks.
        let Some(hunk) = hunk_numbers.get(reloc.target).copied().flatten() else {
            return Err(unsupported(section, reloc));
        };
        let position = by_target.iter().position(|entry| entry.0 == hunk);
        let index = position.unwrap_or_else(|| {
            by_target.push((hunk, Vec::new(), Vec::new()));
            by_target.len() - 1
        });
        if reloc.offset <= MAX_SHORT_OFFSET {
            by_target[index].1.push(reloc.offset);
        } else {
            by_target[index].2.push(reloc.offset);
        }
    }
    by_target.sort_by_key(|entry| entry.0);

    if by_target.iter().any(|entry| !entry.1.is_empty()) {
        out.long(HUNK_RELOC32SHORT);
        let mut words = 0;
        for (hunk, short, _) in by_target.iter().filter(|entry| !entry.1.is_empty()) {
            out.word(short.len() as u16);
            out.word(*hunk as u16);
            for &offset in short {
                out.word(offset as u16);
            }
            words += 2 + short.len();
        }
        out.word(0);
        if (words + 1) % 2 != 0 {
            out.word(0); // Keep the stream long-aligned.
        }
    }
    if by_target.iter().any(|entry| !entry.2.is_empty()) {
        out.long(HUNK_RELOC32);
        for (hunk, _, long) in by_target.iter().filter(|entry| !entry.2.is_empty()) {
            out.long(long.len() as u32);
            out.long(*hunk);
            for &offset in long {
                out.long(offset);
            }
        }
        out.long(0);
    }
    Ok(())
}

fn unsupported(section: &Section, reloc: &Reloc) -> OutputError {
    OutputError::UnsupportedReloc {
        section: section.name.clone(),
        offset: reloc.offset,
        size: reloc.size,
        span: reloc.span,
    }
}

fn write_symbols(assembly: &Assembly, section: usize, out: &mut Writer) {
    let mut symbols = assembly
        .symbols
        .iter()
        .filter(|symbol| symbol.section == Some(section))
        .peekable();
    if symbols.peek().is_none() {
        return;
    }
    out.long(HUNK_SYMBOL);
    for symbol in symbols {
        let name = symbol.name.as_bytes();
        out.long((name.len() as u32).div_ceil(4));
        out.bytes.extend_from_slice(name);
        out.bytes
            .extend(std::iter::repeat(0).take((4 - name.len() % 4) % 4));
        out.long(symbol.value as u32);
    }
    out.long(0);
}
