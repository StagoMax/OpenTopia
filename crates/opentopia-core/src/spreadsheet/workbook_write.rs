use super::{
    cell_value_from_data, contains_invalid_xml_character, ensure_sheet_count,
    ensure_workbook_cell_count, invalid_cell_value, invalid_workbook_coordinate,
    open_workbook_reader, sheet_info, validate_address, validate_write_request, worksheet_formulas,
    worksheet_values, BTreeMap, CellAddress, Data, Formula, Path, Range, Reader, SheetKind,
    SheetVisibility, SheetWriteRequest, SpreadsheetCellInput, SpreadsheetCellValue,
    SpreadsheetError, SpreadsheetFileFormat, SpreadsheetWriteBackend, Workbook, Worksheet,
    WriteWorkbookRequest, WriteWorkbookResult, MAX_FORMULA_BYTES, MAX_INPUT_FILE_BYTES,
    MAX_OUTPUT_FILE_BYTES, MAX_WORKBOOK_CELLS,
};
use crate::delimited;
use std::fs::{self, File};

#[derive(Debug, Default)]
pub(super) struct LoadedWorkbook {
    pub(super) sheets: Vec<LoadedSheet>,
}

#[derive(Debug)]
pub(super) struct LoadedSheet {
    pub(super) name: String,
    pub(super) visibility: SheetVisibility,
    pub(super) cells: BTreeMap<CellAddress, StoredCell>,
}

#[derive(Debug)]
pub(super) struct StoredCell {
    pub(super) value: SpreadsheetCellValue,
    pub(super) formula: Option<String>,
    pub(super) formula_result: Option<String>,
}

pub(super) fn load_workbook(path: &Path) -> Result<LoadedWorkbook, SpreadsheetError> {
    let (mut workbook, _) = open_workbook_reader(path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    let mut loaded = LoadedWorkbook {
        sheets: Vec::with_capacity(metadata.len()),
    };
    let mut workbook_cells = 0usize;

    for metadata in metadata {
        let info = sheet_info(&metadata);
        if info.kind != SheetKind::Worksheet {
            return Err(SpreadsheetError::UnsupportedSheetType {
                sheet: info.name,
                kind: info.kind,
            });
        }
        let values = worksheet_values(&mut workbook, path, &info.name)?;
        let formulas = worksheet_formulas(&mut workbook, path, &info.name)?;
        let mut cells = BTreeMap::new();
        load_values(&values, &info.name, &mut cells)?;
        load_formulas(&formulas, &info.name, &mut cells)?;
        workbook_cells =
            workbook_cells
                .checked_add(cells.len())
                .ok_or(SpreadsheetError::TooManyCells {
                    context: "workbook",
                    actual: usize::MAX,
                    limit: MAX_WORKBOOK_CELLS,
                })?;
        ensure_workbook_cell_count(workbook_cells)?;
        loaded.sheets.push(LoadedSheet {
            name: info.name,
            visibility: info.visibility,
            cells,
        });
    }
    Ok(loaded)
}

pub(super) fn load_spreadsheet(path: &Path) -> Result<LoadedWorkbook, SpreadsheetError> {
    let format = SpreadsheetFileFormat::from_path(path).ok_or_else(|| {
        SpreadsheetError::UnsupportedFormat {
            path: path.to_path_buf(),
            extension: path
                .extension()
                .and_then(|extension| extension.to_str())
                .map(str::to_string),
        }
    })?;
    if format.is_delimited() {
        load_delimited(path, format)
    } else {
        load_workbook(path)
    }
}

fn load_delimited(
    path: &Path,
    format: SpreadsheetFileFormat,
) -> Result<LoadedWorkbook, SpreadsheetError> {
    let metadata = fs::metadata(path).map_err(|source| SpreadsheetError::Io {
        operation: "inspect",
        path: path.to_path_buf(),
        source,
    })?;
    if metadata.len() > MAX_INPUT_FILE_BYTES {
        return Err(SpreadsheetError::FileTooLarge {
            path: path.to_path_buf(),
            actual_bytes: metadata.len(),
            limit_bytes: MAX_INPUT_FILE_BYTES,
        });
    }
    let file = File::open(path).map_err(|source| SpreadsheetError::Io {
        operation: "open",
        path: path.to_path_buf(),
        source,
    })?;
    let delimiter = if format == SpreadsheetFileFormat::Tsv {
        b'\t'
    } else {
        b','
    };
    let mut reader = delimited::byte_reader(file, delimiter);
    let mut cells = BTreeMap::new();
    for (row, record) in reader.byte_records().enumerate() {
        let row = u32::try_from(row).map_err(|_| SpreadsheetError::TooManyCells {
            context: "delimited rows",
            actual: usize::MAX,
            limit: super::EXCEL_MAX_ROWS as usize,
        })?;
        let record = record.map_err(|error| SpreadsheetError::InvalidDelimited {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
        for (column, field) in record.iter().enumerate() {
            let column = u32::try_from(column).map_err(|_| SpreadsheetError::TooManyCells {
                context: "delimited columns",
                actual: usize::MAX,
                limit: super::EXCEL_MAX_COLUMNS as usize,
            })?;
            let address = CellAddress { row, column };
            validate_address(address)?;
            let value = delimited::decode_field(field, row == 0 && column == 0, false);
            if value.is_empty() {
                continue;
            }
            cells.insert(
                address,
                StoredCell {
                    value: SpreadsheetCellValue::String(value),
                    formula: None,
                    formula_result: None,
                },
            );
            ensure_workbook_cell_count(cells.len())?;
        }
    }
    Ok(LoadedWorkbook {
        sheets: vec![LoadedSheet {
            name: "Data".to_string(),
            visibility: SheetVisibility::Visible,
            cells,
        }],
    })
}

fn load_values(
    values: &Range<Data>,
    sheet: &str,
    cells: &mut BTreeMap<CellAddress, StoredCell>,
) -> Result<(), SpreadsheetError> {
    let Some((base_row, base_column)) = values.start() else {
        return Ok(());
    };
    for (relative_row, relative_column, value) in values.used_cells() {
        let address =
            absolute_address(base_row, base_column, relative_row, relative_column, sheet)?;
        let value = cell_value_from_data(value, sheet, address.row, address.column)?;
        cells.insert(
            address,
            StoredCell {
                formula_result: formula_result_from_value(&value),
                value,
                formula: None,
            },
        );
        ensure_workbook_cell_count(cells.len())?;
    }
    Ok(())
}

fn load_formulas(
    formulas: &Range<String>,
    sheet: &str,
    cells: &mut BTreeMap<CellAddress, StoredCell>,
) -> Result<(), SpreadsheetError> {
    let Some((base_row, base_column)) = formulas.start() else {
        return Ok(());
    };
    for (relative_row, relative_column, formula) in formulas.used_cells() {
        let address =
            absolute_address(base_row, base_column, relative_row, relative_column, sheet)?;
        validate_source_formula(formula, sheet, address)?;
        let cell = cells.entry(address).or_insert_with(|| StoredCell {
            value: SpreadsheetCellValue::Empty,
            formula: None,
            formula_result: None,
        });
        cell.formula = Some(formula.clone());
        ensure_workbook_cell_count(cells.len())?;
    }
    Ok(())
}

fn absolute_address(
    base_row: u32,
    base_column: u32,
    relative_row: usize,
    relative_column: usize,
    sheet: &str,
) -> Result<CellAddress, SpreadsheetError> {
    let relative_row =
        u32::try_from(relative_row).map_err(|_| invalid_workbook_coordinate(sheet))?;
    let relative_column =
        u32::try_from(relative_column).map_err(|_| invalid_workbook_coordinate(sheet))?;
    let row = base_row
        .checked_add(relative_row)
        .ok_or_else(|| invalid_workbook_coordinate(sheet))?;
    let column = base_column
        .checked_add(relative_column)
        .ok_or_else(|| invalid_workbook_coordinate(sheet))?;
    let address = CellAddress { row, column };
    validate_address(address)?;
    Ok(address)
}

fn validate_source_formula(
    formula: &str,
    sheet: &str,
    address: CellAddress,
) -> Result<(), SpreadsheetError> {
    if formula.len() > MAX_FORMULA_BYTES {
        return Err(invalid_cell_value(
            sheet,
            address.row,
            address.column,
            format!("source formula exceeds {MAX_FORMULA_BYTES} bytes"),
        ));
    }
    if contains_invalid_xml_character(formula) {
        return Err(invalid_cell_value(
            sheet,
            address.row,
            address.column,
            "source formula contains an unsupported control character",
        ));
    }
    Ok(())
}

pub(super) fn apply_sheet_updates(
    workbook: &mut LoadedWorkbook,
    requests: &[SheetWriteRequest],
) -> Result<(), SpreadsheetError> {
    for request in requests {
        let index = workbook
            .sheets
            .iter()
            .position(|sheet| sheet.name.to_lowercase() == request.name.to_lowercase());
        let index = match index {
            Some(index) => index,
            None => {
                workbook.sheets.push(LoadedSheet {
                    name: request.name.clone(),
                    visibility: request.visibility.unwrap_or_default(),
                    cells: BTreeMap::new(),
                });
                workbook.sheets.len() - 1
            }
        };
        let sheet = &mut workbook.sheets[index];
        if let Some(visibility) = request.visibility {
            sheet.visibility = visibility;
        }
        for update in &request.cells {
            match &update.value {
                SpreadsheetCellInput::Blank => {
                    sheet.cells.remove(&update.address);
                }
                SpreadsheetCellInput::String(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::String(value.clone()),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Integer(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Integer(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Number(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Number(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Boolean(value) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Boolean(*value),
                            formula: None,
                            formula_result: None,
                        },
                    );
                }
                SpreadsheetCellInput::Formula(formula) => {
                    sheet.cells.insert(
                        update.address,
                        StoredCell {
                            value: SpreadsheetCellValue::Empty,
                            formula: Some(formula.expression.clone()),
                            formula_result: formula.cached_result.clone(),
                        },
                    );
                }
            }
        }
    }
    Ok(())
}

pub(super) fn render_workbook(
    loaded: &LoadedWorkbook,
    output: &Path,
) -> Result<Vec<u8>, SpreadsheetError> {
    let mut workbook = Workbook::new();
    for sheet in &loaded.sheets {
        let worksheet = workbook.add_worksheet();
        worksheet
            .set_name(&sheet.name)
            .map_err(|error| write_failed(output, error))?;
        match sheet.visibility {
            SheetVisibility::Visible => {}
            SheetVisibility::Hidden => {
                worksheet.set_hidden(true);
            }
            SheetVisibility::VeryHidden => {
                worksheet.set_very_hidden(true);
            }
        }
        for (address, cell) in &sheet.cells {
            write_stored_cell(worksheet, *address, cell, output)?;
        }
    }
    workbook
        .save_to_buffer()
        .map_err(|error| write_failed(output, error))
}

fn write_stored_cell(
    worksheet: &mut Worksheet,
    address: CellAddress,
    cell: &StoredCell,
    output: &Path,
) -> Result<(), SpreadsheetError> {
    let row = address.row;
    let column = address.column as u16;
    if let Some(expression) = &cell.formula {
        let mut formula = Formula::new(expression);
        if let Some(result) = cell
            .formula_result
            .clone()
            .or_else(|| formula_result_from_value(&cell.value))
        {
            formula = formula.set_result(result);
        }
        worksheet
            .write_formula(row, column, formula)
            .map_err(|error| write_failed(output, error))?;
        return Ok(());
    }

    match &cell.value {
        SpreadsheetCellValue::Empty => {}
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => {
            worksheet
                .write_string(row, column, value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Integer(value) => {
            worksheet
                .write(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Number(value) => {
            worksheet
                .write_number(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::Boolean(value) => {
            worksheet
                .write_boolean(row, column, *value)
                .map_err(|error| write_failed(output, error))?;
        }
        SpreadsheetCellValue::DateTime(value) => {
            worksheet
                .write_number(row, column, value.serial)
                .map_err(|error| write_failed(output, error))?;
        }
    }
    Ok(())
}

fn formula_result_from_value(value: &SpreadsheetCellValue) -> Option<String> {
    match value {
        SpreadsheetCellValue::Empty => None,
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => Some(value.clone()),
        SpreadsheetCellValue::Integer(value) => Some(value.to_string()),
        SpreadsheetCellValue::Number(value) => Some(value.to_string()),
        SpreadsheetCellValue::Boolean(value) => {
            Some(if *value { "TRUE" } else { "FALSE" }.to_string())
        }
        SpreadsheetCellValue::DateTime(value) => Some(value.serial.to_string()),
    }
}

pub(super) fn write_delimited_workbook(
    request: &WriteWorkbookRequest,
    format: SpreadsheetFileFormat,
) -> Result<WriteWorkbookResult, SpreadsheetError> {
    let applied_updates = validate_write_request(request)?;
    let mut workbook = match request.source.as_deref() {
        Some(source) => load_spreadsheet(source)?,
        None => LoadedWorkbook::default(),
    };
    apply_sheet_updates(&mut workbook, &request.sheets)?;
    if workbook.sheets.is_empty() {
        return Err(SpreadsheetError::NoSheets);
    }
    if workbook.sheets.len() != 1 {
        return Err(SpreadsheetError::ValidationFailed {
            message: "CSV/TSV output requires exactly one worksheet; use export_delimited to choose a sheet from a multi-sheet workbook".to_string(),
        });
    }
    let sheet = &workbook.sheets[0];
    if sheet.visibility != SheetVisibility::Visible {
        return Err(SpreadsheetError::NoVisibleSheet);
    }
    let delimiter = if format == SpreadsheetFileFormat::Tsv {
        b'\t'
    } else {
        b','
    };
    let range = used_cell_bounds(&sheet.cells).unwrap_or((
        CellAddress { row: 0, column: 0 },
        CellAddress { row: 0, column: 0 },
    ));
    let mut writer = delimited::writer(Vec::new(), delimiter);
    for row in range.0.row..=range.1.row {
        let record = (range.0.column..=range.1.column)
            .map(|column| {
                sheet
                    .cells
                    .get(&CellAddress { row, column })
                    .map(delimited_cell_text)
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>();
        writer
            .write_record(&record)
            .map_err(|error| write_failed(&request.output, error))?;
    }
    writer
        .flush()
        .map_err(|error| write_failed(&request.output, error))?;
    let bytes = writer
        .into_inner()
        .map_err(|error| write_failed(&request.output, error.error()))?;
    let bytes_written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }
    fs::write(&request.output, bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    let output_cells = sheet.cells.len();
    let result = WriteWorkbookResult {
        output: request.output.clone(),
        bytes_written,
        sheet_count: 1,
        output_cells,
        applied_updates,
        rebuilt_from_source: request.source.is_some(),
        preserved_template_parts: false,
        backend: SpreadsheetWriteBackend::Delimited,
    };
    super::ensure_return_size(&result)?;
    Ok(result)
}

fn used_cell_bounds(
    cells: &BTreeMap<CellAddress, StoredCell>,
) -> Option<(CellAddress, CellAddress)> {
    let mut addresses = cells.keys();
    let first = *addresses.next()?;
    let mut start = first;
    let mut end = first;
    for address in addresses {
        start.row = start.row.min(address.row);
        start.column = start.column.min(address.column);
        end.row = end.row.max(address.row);
        end.column = end.column.max(address.column);
    }
    Some((start, end))
}

fn delimited_cell_text(cell: &StoredCell) -> String {
    if let Some(formula) = &cell.formula {
        return if formula.starts_with('=') {
            formula.clone()
        } else {
            format!("={formula}")
        };
    }
    formula_result_from_value(&cell.value).unwrap_or_default()
}

pub(super) fn write_failed(output: &Path, error: impl std::fmt::Display) -> SpreadsheetError {
    SpreadsheetError::WriteFailed {
        path: output.to_path_buf(),
        message: error.to_string(),
    }
}
