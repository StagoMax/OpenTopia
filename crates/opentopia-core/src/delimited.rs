use std::io::{Read, Write};

/// Builds the crate-wide CSV/TSV reader. Keeping these options in one place
/// prevents preview and spreadsheet execution from disagreeing about quoted
/// newlines, duplicate headers, or ragged records.
pub(crate) fn byte_reader<R: Read>(source: R, delimiter: u8) -> csv::Reader<R> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_reader(source)
}

pub(crate) fn writer<W: Write>(target: W, delimiter: u8) -> csv::Writer<W> {
    csv::WriterBuilder::new()
        .has_headers(false)
        .delimiter(delimiter)
        .from_writer(target)
}

pub(crate) fn decode_field(field: &[u8], strip_utf8_bom: bool, rstrip_tabs: bool) -> String {
    let field = if strip_utf8_bom {
        field.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(field)
    } else {
        field
    };
    let decoded = String::from_utf8_lossy(field);
    if rstrip_tabs {
        decoded.trim_end_matches('\t').to_string()
    } else {
        decoded.into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reader_handles_duplicate_headers_commas_and_quoted_newlines() {
        let input = b"name,name,note\nalpha,beta,\"comma, and\nnewline\"\n";
        let rows = byte_reader(input.as_slice(), b',')
            .byte_records()
            .map(|record| record.expect("valid record"))
            .collect::<Vec<_>>();

        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].get(0), Some(b"name".as_slice()));
        assert_eq!(rows[0].get(1), Some(b"name".as_slice()));
        assert_eq!(rows[1].get(2), Some(b"comma, and\nnewline".as_slice()));
    }

    #[test]
    fn field_cleanup_is_explicit() {
        assert_eq!(decode_field(b"\xef\xbb\xbfvalue\t", true, false), "value\t");
        assert_eq!(decode_field(b"value\t\t", false, true), "value");
    }
}
