use super::{
    fs, invalid_workbook_coordinate, write_failed, BTreeMap, BytesEnd, BytesStart, BytesText,
    CellUpdate, Cursor, Path, PathBuf, Read, SheetWriteRequest, SimpleFileOptions,
    SpreadsheetCellInput, SpreadsheetError, Write, XmlEvent, XmlReader, XmlWriter, ZipArchive,
    ZipWriter,
};

pub(super) fn patch_workbook_template(
    source: &Path,
    sheets: &[SheetWriteRequest],
    output: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let source_bytes = fs::read(source).map_err(|source_error| SpreadsheetError::Io {
        operation: "read",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    let mut archive = ZipArchive::new(Cursor::new(source_bytes)).map_err(|error| {
        SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("invalid XLSX package: {error}"),
        }
    })?;
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

fn read_zip_part(
    archive: &mut ZipArchive<Cursor<Vec<u8>>>,
    name: &str,
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut part = archive
        .by_name(name)
        .map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: source.to_path_buf(),
            message: format!("missing XLSX part {name}: {error}"),
        })?;
    let mut bytes = Vec::new();
    part.read_to_end(&mut bytes)
        .map_err(|source_error| SpreadsheetError::Io {
            operation: "read",
            path: source.to_path_buf(),
            source: source_error,
        })?;
    Ok(bytes)
}

fn workbook_sheet_relationships(
    xml: &[u8],
    source: &Path,
) -> Result<Vec<(String, String)>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut relationships = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "workbook.xml", error))?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"sheet" =>
            {
                let name = xml_attribute(&reader, &element, b"name")?;
                let relationship_id = xml_attribute(&reader, &element, b"id")?;
                if let (Some(name), Some(relationship_id)) = (name, relationship_id) {
                    relationships.push((name, relationship_id));
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(relationships)
}

fn workbook_relationship_targets(
    xml: &[u8],
    source: &Path,
) -> Result<BTreeMap<String, String>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut targets = BTreeMap::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_template_xml(source, "workbook.xml.rels", error))?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"Relationship" =>
            {
                let id = xml_attribute(&reader, &element, b"Id")?;
                let target = xml_attribute(&reader, &element, b"Target")?;
                if let (Some(id), Some(target)) = (id, target) {
                    targets.insert(id, target);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(targets)
}

fn xml_attribute(
    reader: &XmlReader<&[u8]>,
    element: &BytesStart<'_>,
    name: &[u8],
) -> Result<Option<String>, SpreadsheetError> {
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| SpreadsheetError::InvalidWorkbook {
            path: PathBuf::from("<xlsx-xml>"),
            message: error.to_string(),
        })?;
        if attribute.key.local_name().as_ref() != name {
            continue;
        }
        return attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map(|value| Some(value.into_owned()))
            .map_err(|error| SpreadsheetError::InvalidWorkbook {
                path: PathBuf::from("<xlsx-xml>"),
                message: error.to_string(),
            });
    }
    Ok(None)
}

fn normalize_workbook_part_target(target: &str) -> String {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{}", target.trim_start_matches("../"))
    }
}

fn patch_worksheet_xml(
    xml: &[u8],
    updates: &[CellUpdate],
    source: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
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
            XmlEvent::Start(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(element.into_owned()))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                patch_sheet_data(&mut reader, &mut writer, &mut pending, source)?;
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"sheetData" => {
                found_sheet_data = true;
                writer
                    .write_event(XmlEvent::Start(BytesStart::new("sheetData")))
                    .map_err(|error| invalid_template_xml(source, "worksheet", error))?;
                write_remaining_rows(&mut writer, &mut pending, source)?;
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

fn patch_sheet_data(
    reader: &mut XmlReader<&[u8]>,
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
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
                write_rows_before(writer, pending, row_number, source)?;
                let row = collect_xml_element(reader, element, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    let patched = patch_row_xml(&row, row_number, &updates, source)?;
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
                write_rows_before(writer, pending, row_number, source)?;
                if let Some(updates) = pending.remove(&row_number) {
                    write_generated_row(writer, row_number, &updates, source)?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "row", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"sheetData" => {
                write_remaining_rows(writer, pending, source)?;
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
                write_cells_before(&mut writer, row, &mut pending, column, source)?;
                let style = xml_attribute(&reader, &element, b"s")?;
                let original = collect_xml_element(&mut reader, element, source)?;
                if let Some(value) = pending.remove(&column) {
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref(),
                        source,
                    )?;
                } else {
                    writer.get_mut().extend_from_slice(&original);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"c" => {
                let column = cell_column_from_element(&reader, &element, source)?;
                write_cells_before(&mut writer, row, &mut pending, column, source)?;
                if let Some(value) = pending.remove(&column) {
                    let style = xml_attribute(&reader, &element, b"s")?;
                    write_generated_cell(
                        &mut writer,
                        row,
                        column,
                        &value,
                        style.as_deref(),
                        source,
                    )?;
                } else {
                    writer
                        .write_event(XmlEvent::Empty(element.into_owned()))
                        .map_err(|error| invalid_template_xml(source, "cell", error))?;
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"row" => {
                write_remaining_cells(&mut writer, row, &mut pending, source)?;
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
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = pending
        .range(..before)
        .map(|(row, _)| *row)
        .collect::<Vec<_>>();
    for row in rows {
        if let Some(updates) = pending.remove(&row) {
            write_generated_row(writer, row, &updates, source)?;
        }
    }
    Ok(())
}

fn write_remaining_rows(
    writer: &mut XmlWriter<Vec<u8>>,
    pending: &mut BTreeMap<u32, BTreeMap<u32, SpreadsheetCellInput>>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let rows = std::mem::take(pending);
    for (row, updates) in rows {
        write_generated_row(writer, row, &updates, source)?;
    }
    Ok(())
}

fn write_generated_row(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    updates: &BTreeMap<u32, SpreadsheetCellInput>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let mut element = BytesStart::new("row");
    let row_number = row.saturating_add(1).to_string();
    element.push_attribute(("r", row_number.as_str()));
    writer
        .write_event(XmlEvent::Start(element))
        .map_err(|error| invalid_template_xml(source, "row", error))?;
    for (column, value) in updates {
        write_generated_cell(writer, row, *column, value, None, source)?;
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
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let columns = pending
        .range(..before)
        .map(|(column, _)| *column)
        .collect::<Vec<_>>();
    for column in columns {
        if let Some(value) = pending.remove(&column) {
            write_generated_cell(writer, row, column, &value, None, source)?;
        }
    }
    Ok(())
}

fn write_remaining_cells(
    writer: &mut XmlWriter<Vec<u8>>,
    row: u32,
    pending: &mut BTreeMap<u32, SpreadsheetCellInput>,
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let cells = std::mem::take(pending);
    for (column, value) in cells {
        write_generated_cell(writer, row, column, &value, None, source)?;
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
    let mut value = column.saturating_add(1);
    let mut letters = Vec::new();
    while value > 0 {
        let remainder = (value - 1) % 26;
        letters.push((b'A' + remainder as u8) as char);
        value = (value - 1) / 26;
    }
    letters.reverse();
    format!(
        "{}{}",
        letters.into_iter().collect::<String>(),
        row.saturating_add(1)
    )
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
