//! Writes an assembly's results as JSON. Hand-written to avoid a serialisation
//! dependency for one small, flat format.

use std::fmt::Write;

use rasm_one::{
    Assembly, Diagnostic, ExpansionKind, LineEntry, SectionKind, Severity, Span, Symbol,
};

/// Writes a list as a JSON array.
fn array<T>(items: &[T], write_item: impl Fn(&T) -> String) -> String {
    let items: Vec<String> = items.iter().map(write_item).collect();
    format!("[{}]", items.join(","))
}

fn string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if u32::from(c) < 0x20 => {
                // Writing to a String cannot fail.
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn optional<T: std::fmt::Display>(value: Option<T>) -> String {
    value.map_or("null".to_string(), |value| value.to_string())
}

fn span(span: &Span) -> String {
    format!(
        r#"{{"file":{},"line":{},"col":{},"len":{}}}"#,
        span.file, span.line, span.col, span.len
    )
}

fn diagnostic(diagnostic: &Diagnostic) -> String {
    let severity = match diagnostic.severity {
        Severity::Error => "error",
        Severity::Warning => "warning",
    };
    let related = array(&diagnostic.related, |related| {
        format!(
            r#"{{"message":{},"span":{}}}"#,
            string(&related.message),
            span(&related.span)
        )
    });
    format!(
        r#"{{"severity":"{severity}","code":{},"message":{},"span":{},"related":{related}}}"#,
        string(diagnostic.code),
        string(&diagnostic.message),
        diagnostic.span.as_ref().map_or("null".to_string(), span),
    )
}

fn symbol(symbol: &Symbol) -> String {
    format!(
        r#"{{"name":{},"section":{},"value":{},"local":{},"exported":{},"span":{}}}"#,
        string(&symbol.name),
        optional(symbol.section),
        symbol.value,
        symbol.local,
        symbol.exported,
        span(&symbol.span),
    )
}

fn line_entry(entry: &LineEntry) -> String {
    format!(
        r#"{{"section":{},"offset":{},"len":{},"span":{},"expansion":{}}}"#,
        entry.section,
        entry.offset,
        entry.len,
        span(&entry.span),
        optional(entry.expansion),
    )
}

/// Writes the whole report. `output_error` says why no output was written.
pub(crate) fn to_json(assembly: &Assembly, output_error: Option<&str>) -> String {
    let sections = array(&assembly.sections, |section| {
        let kind = match section.kind {
            SectionKind::Code => "code",
            SectionKind::Data => "data",
            SectionKind::Bss => "bss",
        };
        format!(
            r#"{{"name":{},"kind":"{kind}","origin":{},"size":{}}}"#,
            string(&section.name),
            optional(section.origin),
            section.data.len(),
        )
    });
    let expansions = array(&assembly.expansions, |expansion| {
        let kind = match expansion.kind {
            ExpansionKind::Include => "include",
            ExpansionKind::Macro => "macro",
            ExpansionKind::Repeat => "repeat",
        };
        format!(
            r#"{{"kind":"{kind}","site":{},"parent":{}}}"#,
            span(&expansion.site),
            optional(expansion.parent),
        )
    });
    format!(
        r#"{{"outputError":{},"files":{},"diagnostics":{},"sections":{sections},"symbols":{},"lineTable":{},"expansions":{expansions}}}"#,
        output_error.map_or("null".to_string(), string),
        array(&assembly.files, |file| string(file)),
        array(&assembly.diagnostics, diagnostic),
        array(&assembly.symbols, symbol),
        array(&assembly.line_table, line_entry),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_string_escapes_json_specials() {
        assert_eq!(string("a\"b\\c\nd\u{1}"), r#""a\"b\\c\nd\u0001""#);
    }

    #[test]
    fn test_optional_is_null_or_value() {
        assert_eq!(optional(None::<u32>), "null");
        assert_eq!(optional(Some(5)), "5");
    }
}
