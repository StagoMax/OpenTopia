use super::ooxml::{
    invalid_ooxml_xml, normalize_workbook_part_target, open_ooxml_package, read_zip_part,
    workbook_relationship_targets, workbook_sheet_relationships, xml_attribute,
};
use super::{CellRange, Data, Path, SpreadsheetError, SpreadsheetFileFormat, XmlEvent, XmlReader};
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DateTimeDisplayKind {
    Date,
    Time,
    DateTime,
    Duration,
}

pub(super) fn date_time_display_kinds(
    path: &Path,
    sheet: &str,
    range: CellRange,
) -> Result<HashMap<(u32, u32), DateTimeDisplayKind>, SpreadsheetError> {
    if !SpreadsheetFileFormat::from_path(path).is_some_and(SpreadsheetFileFormat::is_ooxml) {
        return Ok(HashMap::new());
    }

    let mut archive = open_ooxml_package(path)?;
    let workbook_xml = read_zip_part(&mut archive, "xl/workbook.xml", path)?;
    let relationships_xml = read_zip_part(&mut archive, "xl/_rels/workbook.xml.rels", path)?;
    let styles_xml = read_zip_part(&mut archive, "xl/styles.xml", path)?;
    let sheet_relationships = workbook_sheet_relationships(&workbook_xml, path)?;
    let relationship_targets = workbook_relationship_targets(&relationships_xml, path)?;
    let relationship_id = sheet_relationships
        .iter()
        .find(|(name, _)| name == sheet)
        .map(|(_, id)| id)
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: sheet.to_string(),
        })?;
    let target = relationship_targets.get(relationship_id).ok_or_else(|| {
        SpreadsheetError::InvalidWorkbook {
            path: path.to_path_buf(),
            message: format!(
                "worksheet relationship {relationship_id:?} for sheet {sheet:?} was not found"
            ),
        }
    })?;
    let worksheet_part = normalize_workbook_part_target(target);
    let worksheet_xml = read_zip_part(&mut archive, &worksheet_part, path)?;
    let style_kinds = parse_style_kinds(&styles_xml, path)?;
    parse_worksheet_style_kinds(&worksheet_xml, &style_kinds, range, path)
}

pub(super) fn formatted_cell_value(
    value: &Data,
    display_kind: Option<DateTimeDisplayKind>,
) -> Option<String> {
    match value {
        Data::DateTime(value) if value.is_duration() => Some(format_duration(value.as_f64())),
        Data::DateTime(value) => {
            let (year, month, day, hour, minute, second, millisecond) = value.to_ymd_hms_milli();
            let display_kind = display_kind.unwrap_or_else(|| {
                if value.as_f64().abs() < 1.0 {
                    DateTimeDisplayKind::Time
                } else if hour == 0 && minute == 0 && second == 0 && millisecond == 0 {
                    DateTimeDisplayKind::Date
                } else {
                    DateTimeDisplayKind::DateTime
                }
            });
            match display_kind {
                DateTimeDisplayKind::Date => Some(format!("{year:04}-{month:02}-{day:02}")),
                DateTimeDisplayKind::Time => Some(format_time(hour, minute, second, millisecond)),
                DateTimeDisplayKind::DateTime => Some(format!(
                    "{year:04}-{month:02}-{day:02} {}",
                    format_time(hour, minute, second, millisecond)
                )),
                DateTimeDisplayKind::Duration => Some(format_duration(value.as_f64())),
            }
        }
        Data::DateTimeIso(value) | Data::DurationIso(value) => Some(value.clone()),
        _ => None,
    }
}

fn parse_style_kinds(
    xml: &[u8],
    source: &Path,
) -> Result<Vec<Option<DateTimeDisplayKind>>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut custom_formats = HashMap::<u32, String>::new();
    let mut cell_format_ids = Vec::<u32>::new();
    let mut in_cell_formats = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "xl/styles.xml", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"cellXfs" => {
                in_cell_formats = true;
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"cellXfs" => {
                in_cell_formats = false;
            }
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"numFmt" =>
            {
                let id = xml_attribute(&reader, &element, b"numFmtId")?
                    .and_then(|value| value.parse::<u32>().ok());
                let code = xml_attribute(&reader, &element, b"formatCode")?;
                if let (Some(id), Some(code)) = (id, code) {
                    custom_formats.insert(id, code);
                }
            }
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if in_cell_formats && element.local_name().as_ref() == b"xf" =>
            {
                let id = xml_attribute(&reader, &element, b"numFmtId")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .unwrap_or_default();
                cell_format_ids.push(id);
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }

    Ok(cell_format_ids
        .into_iter()
        .map(|id| number_format_kind(id, custom_formats.get(&id).map(String::as_str)))
        .collect())
}

fn parse_worksheet_style_kinds(
    xml: &[u8],
    style_kinds: &[Option<DateTimeDisplayKind>],
    range: CellRange,
    source: &Path,
) -> Result<HashMap<(u32, u32), DateTimeDisplayKind>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut result = HashMap::new();
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "worksheet", error))?;
        match event {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"c" =>
            {
                let Some(reference) = xml_attribute(&reader, &element, b"r")? else {
                    continue;
                };
                let Some((row, column)) = cell_address(&reference) else {
                    continue;
                };
                if row < range.start.row
                    || row > range.end.row
                    || column < range.start.column
                    || column > range.end.column
                {
                    continue;
                }
                let style_index = xml_attribute(&reader, &element, b"s")?
                    .and_then(|value| value.parse::<usize>().ok())
                    .unwrap_or_default();
                if let Some(kind) = style_kinds.get(style_index).copied().flatten() {
                    result.insert((row, column), kind);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}

fn number_format_kind(id: u32, custom: Option<&str>) -> Option<DateTimeDisplayKind> {
    match id {
        14..=17 => return Some(DateTimeDisplayKind::Date),
        18..=21 | 45 | 47 => return Some(DateTimeDisplayKind::Time),
        22 => return Some(DateTimeDisplayKind::DateTime),
        46 => return Some(DateTimeDisplayKind::Duration),
        _ => {}
    }
    custom.and_then(custom_number_format_kind)
}

fn custom_number_format_kind(format_code: &str) -> Option<DateTimeDisplayKind> {
    let mut has_date = false;
    let mut has_time = false;
    let mut has_month = false;
    let mut duration = false;
    let mut characters = format_code.chars().peekable();
    let mut in_quotes = false;
    while let Some(character) = characters.next() {
        match character {
            '"' => in_quotes = !in_quotes,
            '\\' | '_' | '*' if !in_quotes => {
                characters.next();
            }
            '[' if !in_quotes => {
                let mut bracket = String::new();
                for value in characters.by_ref() {
                    if value == ']' {
                        break;
                    }
                    bracket.push(value.to_ascii_lowercase());
                }
                let bracket = bracket.trim();
                if !bracket.is_empty()
                    && bracket
                        .chars()
                        .all(|value| matches!(value, 'h' | 'm' | 's'))
                {
                    has_time = true;
                    duration = true;
                }
            }
            _ if in_quotes => {}
            'y' | 'Y' | 'd' | 'D' => has_date = true,
            'h' | 'H' | 's' | 'S' => has_time = true,
            'm' | 'M' => has_month = true,
            ';' => break,
            _ => {}
        }
    }
    if duration {
        Some(DateTimeDisplayKind::Duration)
    } else if has_date && has_time {
        Some(DateTimeDisplayKind::DateTime)
    } else if has_date || (has_month && !has_time) {
        Some(DateTimeDisplayKind::Date)
    } else if has_time {
        Some(DateTimeDisplayKind::Time)
    } else {
        None
    }
}

fn cell_address(reference: &str) -> Option<(u32, u32)> {
    let mut column = 0u32;
    let mut letters = 0usize;
    let mut digits = String::new();
    for byte in reference.bytes() {
        if byte.is_ascii_alphabetic() && digits.is_empty() {
            letters += 1;
            column = column
                .checked_mul(26)?
                .checked_add(u32::from(byte.to_ascii_uppercase() - b'A') + 1)?;
        } else if byte.is_ascii_digit() {
            digits.push(char::from(byte));
        } else {
            return None;
        }
    }
    let row = digits.parse::<u32>().ok()?.checked_sub(1)?;
    Some((row, column.checked_sub(1).filter(|_| letters > 0)?))
}

fn format_time(hour: u8, minute: u8, second: u8, millisecond: u16) -> String {
    if millisecond == 0 {
        format!("{hour:02}:{minute:02}:{second:02}")
    } else {
        format!("{hour:02}:{minute:02}:{second:02}.{millisecond:03}")
    }
}

fn format_duration(serial: f64) -> String {
    let total_milliseconds = (serial * 86_400_000.0).round() as i64;
    let sign = if total_milliseconds < 0 { "-" } else { "" };
    let remaining = total_milliseconds.unsigned_abs();
    let hours = remaining / 3_600_000;
    let minutes = remaining % 3_600_000 / 60_000;
    let seconds = remaining % 60_000 / 1_000;
    let milliseconds = remaining % 1_000;
    if milliseconds == 0 {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{seconds:02}.{milliseconds:03}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_common_excel_date_and_time_formats() {
        assert_eq!(
            custom_number_format_kind("yyyy-mm-dd"),
            Some(DateTimeDisplayKind::Date)
        );
        assert_eq!(
            custom_number_format_kind("yyyy-mm-dd hh:mm:ss"),
            Some(DateTimeDisplayKind::DateTime)
        );
        assert_eq!(
            custom_number_format_kind("hh:mm:ss"),
            Some(DateTimeDisplayKind::Time)
        );
        assert_eq!(
            custom_number_format_kind("[h]:mm:ss"),
            Some(DateTimeDisplayKind::Duration)
        );
        assert_eq!(
            custom_number_format_kind("[Magenta]yyyy-mm-dd"),
            Some(DateTimeDisplayKind::Date)
        );
        assert_eq!(custom_number_format_kind("0.00"), None);
    }

    #[test]
    fn parses_absolute_cell_addresses() {
        assert_eq!(cell_address("A1"), Some((0, 0)));
        assert_eq!(cell_address("B16"), Some((15, 1)));
        assert_eq!(cell_address("AA104"), Some((103, 26)));
        assert_eq!(cell_address("1A"), None);
    }
}
