use super::{
    cell_value_from_data, contains_invalid_xml_character, ensure_sheet_count,
    ensure_workbook_cell_count, invalid_cell_value, invalid_workbook_coordinate, open_xlsx,
    sheet_info, validate_address, worksheet_formulas, worksheet_values, BTreeMap, CellAddress,
    Data, Formula, Path, Range, Reader, SheetKind, SheetVisibility, SheetWriteRequest,
    SpreadsheetCellInput, SpreadsheetCellValue, SpreadsheetError, Workbook, Worksheet,
    MAX_FORMULA_BYTES, MAX_WORKBOOK_CELLS,
};

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
    let (mut workbook, _) = open_xlsx(path)?;
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

pub(super) fn write_failed(output: &Path, error: impl std::fmt::Display) -> SpreadsheetError {
    SpreadsheetError::WriteFailed {
        path: output.to_path_buf(),
        message: error.to_string(),
    }
}
