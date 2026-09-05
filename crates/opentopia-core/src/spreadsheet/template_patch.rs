use super::ooxml::{
    normalize_workbook_part_target, open_ooxml_package, read_zip_part,
    workbook_relationship_targets, workbook_sheet_relationships, xml_attribute,
};
use super::{
    format_a1_address, format_a1_range, invalid_workbook_coordinate, parse_a1_address,
    parse_a1_range, write_failed, BTreeMap, BytesEnd, BytesStart, BytesText, CellAddress,
    CellRange, CellUpdate, Cursor, Path, Read, SheetWriteRequest, SimpleFileOptions,
    SpreadsheetCellInput, SpreadsheetError, Write, XmlEvent, XmlReader, XmlWriter, ZipWriter,
};

pub(super) fn patch_workbook_template(
    source: &Path,
    sheets: &[SheetWriteRequest],
    output: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut archive = open_ooxml_package(source)?;
    let workbook_xml = read_zip_part(&mut archive, "xl/workbook.xml", source)?;
    let relationships_xml = read_zip_part(&mut archive, "xl/_rels/workbook.xml.rels", source)?;
    let sheet_relationships = workbook_sheet_relationships(&workbook_xml, source)?;
    let relationship_targets = workbook_relationship_targets(&relationships_xml, source)?;
    let mut updates_by_part = BTreeMap::<String, Vec<CellUpdate>>::new();
    for sheet in sheets {
        let relationship_id = sheet_relationships
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(&sheet.name))
            .map(|(_, id)| id)
            .ok_or_else(|| SpreadsheetError::SheetNotFound {
                sheet: sheet.name.clone(),
            })?;
        let target = relationship_targets.get(relationship_id).ok_or_else(|| {
            SpreadsheetError::InvalidWorkbook {
                path: source.to_path_buf(),
                message: format!(
                    "worksheet relationship {relationship_id:?} for sheet {:?} was not found",
                    sheet.name
                ),
            }
        })?;
        updates_by_part.insert(normalize_workbook_part_target(target), sheet.cells.clone());
    }

    let mut output_cursor = Cursor::new(Vec::new());
    {
        let mut writer = ZipWriter::new(&mut output_cursor);
        for index in 0..archive.len() {
            let (name, is_directory, compression, unix_mode, mut bytes) = {
                let mut file =
                    archive
                        .by_index(index)
                        .map_err(|error| SpreadsheetError::InvalidWorkbook {
                            path: source.to_path_buf(),
                            message: format!("failed to read XLSX package entry {index}: {error}"),
                        })?;
                let name = file.name().to_string();
                let is_directory = file.is_dir();
                let compression = file.compression();
                let unix_mode = file.unix_mode();
                let mut bytes = Vec::new();
                file.read_to_end(&mut bytes)
                    .map_err(|source_error| SpreadsheetError::Io {
                        operation: "read",
                        path: source.to_path_buf(),
                        source: source_error,
                    })?;
                (name, is_directory, compression, unix_mode, bytes)
            };
            if let Some(updates) = updates_by_part.get(&name) {
                bytes = patch_worksheet_xml(&bytes, updates, source)?;
            }
            let mut options = SimpleFileOptions::default().compression_method(compression);
            if let Some(mode) = unix_mode {
                options = options.unix_permissions(mode);
            }
            if is_directory {
                writer
                    .add_directory(&name, options)
                    .map_err(|error| write_failed(output, error))?;
            } else {
                writer
                    .start_file(&name, options)
                    .map_err(|error| write_failed(output, error))?;
                writer
                    .write_all(&bytes)
                    .map_err(|error| write_failed(output, error))?;
            }
        }
        writer
            .finish()
            .map_err(|error| write_failed(output, error))?;
    }
    Ok(output_cursor.into_inner())
}

fn patch_worksheet_xml(
    xml: &[u8],
    updates: &[CellUpdate],
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let update_range = update_bounds(updates);
    let existing_styles = collect_cell_styles(xml, source)?;
    let propagated_styles = updates
        .iter()
        .filter_map(|update| {
            update.style_from.and_then(|style_from| {
                existing_styles
                    .get(&style_from)
                    .cloned()
                    .map(|style| (update.address, style))
            })
        })
        .collect::<BTreeMap<_, _>>();
    let mut pending = BTreeMap::<u32, BTreeMap<u32, SpreadsheetCellInput>>::new();
    for update in updates {
        pending
            .entry(update.address.row)
            .or_default()
            .insert(update.address.column, update.value.clone());
    }
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut writer = XmlWriter::new(Vec::with_capacity(xml.len()));
    let mut found_sheet_data = false;
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
        match event {
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"dimension" => {
                let existing = xml_attribute(&reader, &element, b"ref")?
                    .as_deref()
                    .and_then(|value| parse_a1_range(value).ok());
                writer
                    .write_event(XmlEvent::Empty(dimension_element(merge_ranges(
                        existing,
                        update_range,
                    ))))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"dimension" => {
                let existing = xml_attribute(&reader, &element, b"ref")?
                    .as_deref()
                    .and_then(|value| parse_a1_range(value).ok());
                writer
                    .write_event(XmlEvent::Start(dimension_element(merge_ranges(
                        existing,
                        update_range,
                    ))))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                patch_sheet_data(
                    &mut reader,
                    &mut writer,
                    &mut pending,
                    &propagated_styles,
                    source,
                )?;
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(BytesStart::new("sheetData")))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                write_remaining_rows(&mut writer, &mut pending, &propagated_styles, source)?;
                writer
                    .write_event(XmlEvent::End(BytesEnd::new("sheetData")))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
            }
            XmlEvent::Eof => break,
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "worksheet", error))?,
        }
    }
    if !found_sheet_data {
        return Err(SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: "worksheet is missing sheetData".to_string(),
        });
    }
    Ok(writer.into_inner())
}

fn collect_cell_styles(
    xml: &[u8],
    source: &Path,
) -> Result<BTreeMap<CellAddress, String>, SpreadsheetError> {
    let mut styles = BTreeMap::new();
    let mut reader = XmlReader::from_reader(xml);
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "worksheet styles", error))?;
        match event {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"c" =>
            {
                let reference = xml_attribute(&reader, &element, b"r")?.ok_or_else(|| {
                    SpreadsheetError::InvalidWorkbook {
                        path: source.to_path_buf(),
                        message: "worksheet cell is missing r attribute".to_string(),
                    }
                })?;
                if let Some(style) = xml_attribute(&reader, &element, b"s")? {
                    styles.insert(parse_a1_address(&reference)?, style);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(styles)
}

fn update_bounds(updates: &[CellUpdate]) -> Option<CellRange> {
    let mut addresses = updates.iter().map(|update| update.address);
    let first = addresses.next()?;
    Some(addresses.fold(
        CellRange {
            start: first,
            end: first,
        },
        |range, address| CellRange {
            start: CellAddress {
                row: range.start.row.min(address.row),
                column: range.start.column.min(address.column),
            },
            end: CellAddress {
                row: range.end.row.max(address.row),
                column: range.end.column.max(address.column),
            },
        },
    ))
}

fn merge_ranges(left: Option<CellRange>, right: Option<CellRange>) -> Option<CellRange> {
    match (left, right) {
        (Some(left), Some(right)) => Some(CellRange {
            start: CellAddress {
                row: left.start.row.min(right.start.row),
                column: left.start.column.min(right.start.column),
            },
            end: CellAddress {
                row: left.end.row.max(right.end.row),
                column: left.end.column.max(right.end.column),
            },
        }),
        (left, right) => left.or(right),
    }
}

fn dimension_element(range: Option<CellRange>) -> BytesStart<'static> {
    let mut element = BytesStart::new("dimension");
    let reference = range
        .map(format_a1_range)
        .unwrap_or_else(|| "A1".to_string());
    element.push_attribute(("ref", reference.as_str()));
    element.into_owned()
}

fn patch_sheet_data(
    reader: &mut XmlReader<&[u8]>,
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "sheetData", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"row" => {
                let row_number = xml_attribute(reader, &element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| SpreadsheetError::InvalidWorkbook {
                        path: source.to_path_buf(),
                        message: "worksheet row is missing a valid one-based r attribute"
                            .to_string(),
                    })?;
                write_rows_before(writer, pending, row_number, propagated_styles, source)?;
                let row = collect_xml_element(reader, element, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    let patched =
                        patch_row_xml(&row, row_number, &updates, propagated_styles, source)?;
                    writer.get_mut().extend_from_slice(&patched);
                } else {
                    writer.get_mut().extend_from_slice(&row);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"row" => {
                let row_number = xml_attribute(reader, &element, b"r")?
                    .and_then(|value| value.parse::<u32>().ok())
                    .and_then(|value| value.checked_sub(1))
                    .ok_or_else(|| SpreadsheetError::InvalidWorkbook {
                        path: source.to_path_buf(),
                        message: "worksheet row is missing a valid one-based r attribute"
                            .to_string(),
                    })?;
                write_rows_before(writer, pending, row_number, propagated_styles, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    write_generated_row(writer, row_number, &updates, propagated_styles, source)?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "row", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"sheetData" => {
                write_remaining_rows(writer, pending, propagated_styles, source)?;
                writer
                    .write_event(XmlEvent::End(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "sheetData", error))?;
                return Ok(());
            }
            XmlEvent::Eof => {
                return Err(SpreadsheetError::InvalidWorkbook {
                    path: source.to_path_buf(),
                    message: "worksheet sheetData ended unexpectedly".to_string(),
                });
            }
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "sheetData", error))?,
        }
    }
}

fn collect_xml_element(
    reader: &mut XmlReader<&[u8]>,
    start: BytesStart<'_>,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut writer = XmlWriter::new(Vec::new());
    writer
        .write_event(XmlEvent::Start(start.into_owned()))
        .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
    let mut depth = 1usize;
    while depth > 0 {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
        match &event {
            XmlEvent::Start(_) => depth += 1,
            XmlEvent::End(_) => depth -= 1,
            XmlEvent::Eof => {
                return Err(SpreadsheetError::InvalidWorkbook {
                    path: source.to_path_buf(),
                    message: "worksheet element ended unexpectedly".to_string(),
                });
            }
            _ => {}
        }
        writer
            .write_event(event.into_owned())
            .map_err(|error| invalid_template_xml(source, "worksheet element", error))?;
    }
    Ok(writer.into_inner())
}

fn patch_row_xml(
    row_xml: &[u8],
    row: u32,
    updates: &BTreeMap<u32, SpreadsheetCellInput>,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut pending = updates.clone();
    let mut reader = XmlReader::from_reader(row_xml);
    reader.config_mut().trim_text(false);
    let mut writer = XmlWriter::new(Vec::with_capacity(row_xml.len()));
    loop {
        let event = reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "row", error))?;
        match event {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"c" => {
                let column = cell_column_from_element(&reader, &element, source)?;
                write_cells_before(
                    &mut writer,
                    row,
                    &mut pending,
                    column,
                    propagated_styles,
                    source,
                )?;
                let style = xml_attribute(&reader, &element, b"s")?;
                let original = collect_xml_element(&mut reader, element, source)?;
                if let Some(value) = pending.remove(&column) {
                    let propagated = propagated_styles
                        .get(&CellAddress { row, column })
                        .map(String::as_str);
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref().or(propagated),
                        source,
                    )?;
                } else {
                    writer.get_mut().extend_from_slice(&original);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"c" => {
                let column = cell_column_from_element(&reader, &element, source)?;
                write_cells_before(
                    &mut writer,
                    row,
                    &mut pending,
                    column,
                    propagated_styles,
                    source,
                )?;
                if let Some(value) = pending.remove(&column) {
                    let style = xml_attribute(&reader, &element, b"s")?;
                    let propagated = propagated_styles
                        .get(&CellAddress { row, column })
                        .map(String::as_str);
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref().or(propagated),
                        source,
                    )?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "cell", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"row" => {
                write_remaining_cells(&mut writer, row, &mut pending, propagated_styles, source)?;
                writer
                    .write_event(XmlEvent::End(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "row", error))?;
            }
            XmlEvent::Eof => break,
            event => writer
                .write_event(event.into_owned())
                .map_err(|error| invalid_template_xml(source, "row", error))?,
        }
    }
    Ok(writer.into_inner())
}

fn cell_column_from_element(
    reader: &XmlReader<&[u8]>,
    element: &BytesStart<'_>,
    source: &Path,
) -> Result<u32, SpreadsheetError> {
    let reference =
        xml_attribute(reader, element, b"r")?.ok_or_else(|| SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: "worksheet cell is missing r attribute".to_string(),
        })?;
    let mut column = 0u32;
    let mut letters = 0usize;
    for byte in reference.bytes() {
        if !byte.is_ascii_alphabetic() {
            break;
        }
        letters += 1;
        column = column
            .checked_mul(26)
            .and_then(|value| value.checked_add(u32::from(byte.to_ascii_uppercase() - b'A') + 1))
            .ok_or_else(|| invalid_workbook_coordinate("worksheet"))?;
    }
    if letters == 0 || column == 0 {
        return Err(SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("invalid worksheet cell reference {reference:?}"),
        });
    }
    Ok(column - 1)
}

fn write_rows_before(
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    before: u32,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = pending
        .range(..before)
        .map(|(row, _)| *row)
        .collect::<Vec<_>>();
    for row in rows {
        if let Some(updates) = pending.remove(&row) {
            write_generated_row(writer, row, &updates, propagated_styles, source)?;
        }
    }
    Ok(())
}

fn write_remaining_rows(
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = std::mem::take(pending);
    for (row, updates) in rows {
        write_generated_row(writer, row, &updates, propagated_styles, source)?;
    }
    Ok(())
}

fn write_generated_row(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    updates: &BTreeMap<u32, SpreadsheetCellInput>,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let mut element = BytesStart::new("row");
    let row_number = row.saturating_add(1).to_string();
    element.push_attribute(("r", row_number.as_str()));
    writer
        .write_event(XmlEvent::Start(element))
        .map_err(|error| invalid_template_xml(source, "row", error))?;
    for (column, value) in updates {
        let style = propagated_styles
            .get(&CellAddress {
                row,
                column: *column,
            })
            .map(String::as_str);
        write_generated_cell(writer, row, *column, value, style, source)?;
    }
    writer
        .write_event(XmlEvent::End(BytesEnd::new("row")))
        .map_err(|error| invalid_template_xml(source, "row", error))?;
    Ok(())
}

fn write_cells_before(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    pending: &mut BTreeMap<u32, SpreadsheetCellInput>,
    before: u32,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let columns = pending
        .range(..before)
        .map(|(column, _)| *column)
        .collect::<Vec<_>>();
    for column in columns {
        if let Some(value) = pending.remove(&column) {
            let style = propagated_styles
                .get(&CellAddress { row, column })
                .map(String::as_str);
            write_generated_cell(writer, row, column, &value, style, source)?;
        }
    }
    Ok(())
}

fn write_remaining_cells(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    pending: &mut BTreeMap<u32, SpreadsheetCellInput>,
    propagated_styles: &BTreeMap<CellAddress, String>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let cells = std::mem::take(pending);
    for (column, value) in cells {
        let style = propagated_styles
            .get(&CellAddress { row, column })
            .map(String::as_str);
        write_generated_cell(writer, row, column, &value, style, source)?;
    }
    Ok(())
}

fn write_generated_cell(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    column: u32,
    value: &SpreadsheetCellInput,
    style: Option<&str>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let reference = cell_reference(row, column);
    let mut cell = BytesStart::new("c");
    cell.push_attribute(("r", reference.as_str()));
    if let Some(style) = style {
        cell.push_attribute(("s", style));
    }
    match value {
        SpreadsheetCellInput::Blank => {
            if style.is_some() {
                writer
                    .write_event(XmlEvent::Empty(cell))
                    .map_err(|error| invalid_template_xml(source, "cell", error))?;
            }
        }
        SpreadsheetCellInput::String(value) => {
            cell.push_attribute(("t", "inlineStr"));
            writer
                .write_event(XmlEvent::Start(cell))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Start(BytesStart::new("is")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            let mut text = BytesStart::new("t");
            text.push_attribute(("xml:space", "preserve"));
            writer
                .write_event(XmlEvent::Start(text))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Text(BytesText::new(value)))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::End(BytesEnd::new("t")))
                .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("is"))))
                .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("c"))))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
        }
        SpreadsheetCellInput::Integer(value) => {
            write_scalar_cell(writer, cell, &value.to_string(), source)?;
        }
        SpreadsheetCellInput::Number(value) => {
            write_scalar_cell(writer, cell, &value.to_string(), source)?;
        }
        SpreadsheetCellInput::Boolean(value) => {
            cell.push_attribute(("t", "b"));
            write_scalar_cell(writer, cell, if *value { "1" } else { "0" }, source)?;
        }
        SpreadsheetCellInput::Formula(formula) => {
            writer
                .write_event(XmlEvent::Start(cell))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Start(BytesStart::new("f")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::Text(BytesText::new(
                    formula.expression.trim_start_matches('='),
                )))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            writer
                .write_event(XmlEvent::End(BytesEnd::new("f")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
            if let Some(result) = &formula.cached_result {
                writer
                    .write_event(XmlEvent::Start(BytesStart::new("v")))
                    .and_then(|_| writer.write_event(XmlEvent::Text(BytesText::new(result))))
                    .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("v"))))
                    .map_err(|error| invalid_template_xml(source, "cell", error))?;
            }
            writer
                .write_event(XmlEvent::End(BytesEnd::new("c")))
                .map_err(|error| invalid_template_xml(source, "cell", error))?;
        }
    }
    Ok(())
}

fn write_scalar_cell(
    writer: &mut XmlWriter<Vec<u8>>,
    cell: BytesStart<'_>,
    value: &str,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    writer
        .write_event(XmlEvent::Start(cell.into_owned()))
        .and_then(|_| writer.write_event(XmlEvent::Start(BytesStart::new("v"))))
        .and_then(|_| writer.write_event(XmlEvent::Text(BytesText::new(value))))
        .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("v"))))
        .and_then(|_| writer.write_event(XmlEvent::End(BytesEnd::new("c"))))
        .map_err(|error| invalid_template_xml(source, "cell", error))?;
    Ok(())
}

fn cell_reference(row: u32, column: u32) -> String {
    format_a1_address(CellAddress { row, column })
}

fn invalid_template_xml(
    source: &Path,
    part: &str,
    error: impl std::fmt::Display,
) -> SpreadsheetError {
    SpreadsheetError::InvalidWorkbook {
        path: source.to_path_buf(),
        message: format!("invalid {part} XML: {error}"),
    }
}
