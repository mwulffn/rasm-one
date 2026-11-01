//! Optional listing file: what each source line became, then the symbols.

use std::fmt::Write;

use crate::{Assembly, LineEntry};

/// Bytes shown per listing line; the rest of a run is summarised.
const BYTES_SHOWN: usize = 8;

/// Writes a listing in address order, one section at a time, so macro and
/// include expansions appear where their bytes are.
pub fn write_listing(assembly: &Assembly) -> String {
    let sources: Vec<Vec<&[u8]>> = assembly
        .sources
        .iter()
        .map(|source| source.split(|&b| b == b'\n').collect())
        .collect();
    let mut out = String::new();
    for (index, section) in assembly.sections.iter().enumerate() {
        // Writing to a String cannot fail.
        let _ = writeln!(
            out,
            "Section {index}: {} ({} bytes)",
            section.name,
            section.data.len()
        );
        for entry in assembly.line_table.iter().filter(|e| e.section == index) {
            write_entry(&mut out, assembly, &sources, entry);
        }
        let _ = writeln!(out);
    }
    write_symbols(&mut out, assembly);
    out
}

fn write_entry(out: &mut String, assembly: &Assembly, sources: &[Vec<&[u8]>], entry: &LineEntry) {
    let start = entry.offset as usize;
    let end = start + entry.len as usize;
    let section = &assembly.sections[entry.section];
    let address = u64::from(section.origin.unwrap_or(0)) + u64::from(entry.offset);
    let shown = &section.data[start..end.min(start + BYTES_SHOWN)];
    let mut bytes: String = shown.iter().fold(String::new(), |mut text, byte| {
        let _ = write!(text, "{byte:02X}");
        text
    });
    if entry.len as usize > BYTES_SHOWN {
        bytes.push_str("..");
    }
    let file = assembly
        .files
        .get(entry.span.file as usize)
        .map_or("?", String::as_str);
    let text = source_line(sources, entry);
    let _ = writeln!(
        out,
        "{address:08X}  {bytes:<18}  {file}:{:<5}  {text}",
        entry.span.line
    );
}

fn source_line(sources: &[Vec<&[u8]>], entry: &LineEntry) -> String {
    let line = sources
        .get(entry.span.file as usize)
        .and_then(|lines| lines.get(entry.span.line as usize - 1))
        .copied()
        .unwrap_or(&[]);
    let line = line.strip_suffix(b"\r").unwrap_or(line);
    String::from_utf8_lossy(line).into_owned()
}

fn write_symbols(out: &mut String, assembly: &Assembly) {
    let _ = writeln!(out, "Symbols");
    let mut symbols: Vec<_> = assembly.symbols.iter().collect();
    symbols.sort_by_key(|symbol| (symbol.section, symbol.value));
    for symbol in symbols {
        let place = match symbol.section {
            Some(section) => section.to_string(),
            None => "abs".to_string(),
        };
        let _ = writeln!(
            out,
            "{:08X}  {place:<4}  {}",
            symbol.value as u32, symbol.name
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::MemoryProvider;
    use crate::{assemble, Options};

    fn listing(source: &str) -> String {
        let mut files = MemoryProvider::new();
        files.insert("t.s", source);
        write_listing(&assemble(&Options::unoptimised("t.s"), &files))
    }

    #[test]
    fn test_write_listing_shows_address_bytes_and_source() {
        let text = listing(" nop\nlab: rts\n");
        assert!(text.contains("00000000  4E71"), "{text}");
        assert!(text.contains("t.s:1"), "{text}");
        assert!(text.contains("00000002  4E75"), "{text}");
        assert!(text.contains("lab: rts"), "{text}");
    }

    #[test]
    fn test_write_listing_summarises_long_runs() {
        let text = listing(" blk.b 9,1\n");
        assert!(text.contains("0101010101010101.."), "{text}");
    }

    #[test]
    fn test_write_listing_includes_symbols_and_org_addresses() {
        let text = listing(" org $1000\nlab: nop\nval equ 5\n");
        assert!(text.contains("00001000  4E71"), "{text}");
        assert!(text.contains("00001000  abs   lab"), "{text}");
        assert!(text.contains("00000005  abs   val"), "{text}");
    }

    #[test]
    fn test_write_listing_shows_included_and_macro_lines() {
        let mut files = MemoryProvider::new();
        files.insert("t.s", "m macro\n rts\n endm\n include \"a.i\"\n m\n");
        files.insert("a.i", " nop ; included\n");
        let text = write_listing(&assemble(&Options::unoptimised("t.s"), &files));
        assert!(text.contains("a.i:1"), "{text}");
        assert!(text.contains("nop ; included"), "{text}");
        assert!(text.contains("t.s:2"), "{text}");
    }

    #[test]
    fn test_write_listing_after_errors_still_works() {
        assert!(listing(" bogus\n nop\n").contains("4E71"));
    }
}
