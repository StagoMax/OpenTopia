use super::display::{date_time_display_kinds, formatted_cell_value};
use super::{
    add_used_positions, cell_value_from_data, collect_sheet_stats, ensure_sheet_count,
    ensure_workbook_cell_count, open_workbook_reader, sheet_info, validate_address,
    validate_read_range, validate_return_text, worksheet_formulas, worksheet_values, CellAddress,
    CellRange, FilterRowsRequest, FilterRowsResult, FindCellsRequest, FindCellsResult,
    InspectWorkbookRequest, InspectWorkbookResult, ListSheetsRequest, ListSheetsResult,
    ReadRangeRequest, ReadRangeResult, ReadRangesRequest, ReadRangesResult, SheetInspection,
    SheetKind, SheetStats, SpreadsheetCell, SpreadsheetCellMatch, SpreadsheetCellValue,
    SpreadsheetError, SpreadsheetFilterCondition, SpreadsheetFilterMatchMode,
    SpreadsheetFilterOperator, SpreadsheetFilterValue, SpreadsheetTextMatchMode,
    MAX_FILTER_CONDITIONS, MAX_FILTER_RESULTS, MAX_FIND_RESULTS, MAX_READ_CELLS, MAX_READ_COLUMNS,
    MAX_READ_RANGES, MAX_READ_ROWS, MAX_WORKBOOK_CELLS,
};
use calamine::{Data, Reader};
use serde::Serialize;
use std::collections::HashSet;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DisplayReadRangeResult {
    pub path: std::path::PathBuf,
    pub sheet: String,
    pub range: CellRange,
    pub rows: Vec<Vec<DisplaySpreadsheetCell>>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DisplaySpreadsheetCell {
    pub value: SpreadsheetCellValue,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formula: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub formatted: Option<String>,
}

pub fn list_sheets(request: &ListSheetsRequest) -> Result<ListSheetsResult, SpreadsheetError> {
    let (workbook, file_size_bytes) = open_workbook_reader(&request.path)?;
    let sheets = workbook
        .sheets_metadata()
        .iter()
        .map(sheet_info)
        .collect::<Vec<_>>();
    ensure_sheet_count(sheets.len())?;

    let result = ListSheetsResult {
        path: request.path.clone(),
        file_size_bytes,
        sheets,
    };
    Ok(result)
}

pub fn inspect_workbook(
    request: &InspectWorkbookRequest,
) -> Result<InspectWorkbookResult, SpreadsheetError> {
    let (mut workbook, file_size_bytes) = open_workbook_reader(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;

    let mut sheets = Vec::with_capacity(metadata.len());
    let mut workbook_cells = 0usize;
    for sheet in metadata {
        let info = sheet_info(&sheet);
        let stats = if info.kind == SheetKind::Worksheet {
            let values = worksheet_values(&mut workbook, &request.path, &info.name)?;
            let formulas = worksheet_formulas(&mut workbook, &request.path, &info.name)?;
            collect_sheet_stats(&values, &formulas, &info.name)?
        } else {
            SheetStats::default()
        };

        workbook_cells = workbook_cells.checked_add(stats.populated_cells).ok_or(
            SpreadsheetError::TooManyCells {
                context: "workbook",
                actual: usize::MAX,
                limit: MAX_WORKBOOK_CELLS,
            },
        )?;
        ensure_workbook_cell_count(workbook_cells)?;
        sheets.push(SheetInspection {
            sheet: info,
            used_range: stats.used_range,
            populated_cells: stats.populated_cells as u64,
        });
    }

    let result = InspectWorkbookResult {
        path: request.path.clone(),
        file_size_bytes,
        sheets,
        populated_cells: workbook_cells as u64,
    };
    Ok(result)
}

pub fn read_range(request: &ReadRangeRequest) -> Result<ReadRangeResult, SpreadsheetError> {
    let loaded = load_range(request, false)?;
    let result = ReadRangeResult {
        path: loaded.path,
        sheet: loaded.sheet,
        range: loaded.range,
        rows: loaded
            .rows
            .into_iter()
            .map(|row| {
                row.into_iter()
                    .map(|cell| SpreadsheetCell {
                        value: cell.value,
                        formula: cell.formula,
                    })
                    .collect()
            })
            .collect(),
    };
    Ok(result)
}

pub(crate) fn read_range_for_display(
    request: &ReadRangeRequest,
) -> Result<DisplayReadRangeResult, SpreadsheetError> {
    let result = load_range(request, true)?;
    Ok(result)
}

fn load_range(
    request: &ReadRangeRequest,
    include_formatted: bool,
) -> Result<DisplayReadRangeResult, SpreadsheetError> {
    validate_read_range(request.range)?;
    let (mut workbook, _) = open_workbook_reader(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    let sheet = metadata
        .iter()
        .find(|sheet| sheet.name == request.sheet)
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.sheet.clone(),
        })?;
    let info = sheet_info(sheet);
    if info.kind != SheetKind::Worksheet {
        return Err(SpreadsheetError::UnsupportedSheetType {
            sheet: info.name,
            kind: info.kind,
        });
    }

    let values = worksheet_values(&mut workbook, &request.path, &request.sheet)?;
    let formulas = worksheet_formulas(&mut workbook, &request.path, &request.sheet)?;
    let stats = collect_sheet_stats(&values, &formulas, &request.sheet)?;
    ensure_workbook_cell_count(stats.populated_cells)?;
    let has_serial_dates = include_formatted
        && (request.range.start.row..=request.range.end.row).any(|row| {
            (request.range.start.column..=request.range.end.column)
                .any(|column| matches!(values.get_value((row, column)), Some(Data::DateTime(_))))
        });
    let display_kinds = if has_serial_dates {
        date_time_display_kinds(&request.path, &request.sheet, request.range).unwrap_or_default()
    } else {
        Default::default()
    };

    let row_count = request.range.row_count().expect("validated range") as usize;
    let column_count = request.range.column_count().expect("validated range") as usize;
    let mut rows = Vec::with_capacity(row_count);
    for row in request.range.start.row..=request.range.end.row {
        let mut cells = Vec::with_capacity(column_count);
        for column in request.range.start.column..=request.range.end.column {
            let value = values.get_value((row, column)).unwrap_or(&Data::Empty);
            let formula = formulas
                .get_value((row, column))
                .filter(|formula| !formula.is_empty())
                .cloned();
            if let Some(formula) = &formula {
                validate_return_text(formula, &request.sheet, row, column)?;
            }
            cells.push(DisplaySpreadsheetCell {
                value: cell_value_from_data(value, &request.sheet, row, column)?,
                formula,
                formatted: if include_formatted {
                    formatted_cell_value(value, display_kinds.get(&(row, column)).copied())
                } else {
                    None
                },
            });
        }
        rows.push(cells);
    }

    Ok(DisplayReadRangeResult {
        path: request.path.clone(),
        sheet: request.sheet.clone(),
        range: request.range,
        rows,
    })
}

pub fn read_ranges(request: &ReadRangesRequest) -> Result<ReadRangesResult, SpreadsheetError> {
    if request.ranges.is_empty() {
        return Err(SpreadsheetError::InvalidRange {
            reason: "read_ranges requires at least one range",
        });
    }
    if request.ranges.len() > MAX_READ_RANGES {
        return Err(SpreadsheetError::TooManyCells {
            context: "read range list",
            actual: request.ranges.len(),
            limit: MAX_READ_RANGES,
        });
    }
    let total_cells = request.ranges.iter().try_fold(0u64, |total, item| {
        validate_read_range(item.range)?;
        total
            .checked_add(item.range.cell_count().expect("validated range"))
            .ok_or(SpreadsheetError::InvalidRange {
                reason: "combined range cell count overflowed",
            })
    })?;
    if total_cells > MAX_READ_CELLS {
        return Err(SpreadsheetError::RangeTooLarge {
            rows: request
                .ranges
                .iter()
                .filter_map(|item| item.range.row_count())
                .sum(),
            columns: request
                .ranges
                .iter()
                .filter_map(|item| item.range.column_count())
                .sum(),
            cells: total_cells,
            max_rows: MAX_READ_ROWS,
            max_columns: MAX_READ_COLUMNS,
            max_cells: MAX_READ_CELLS,
        });
    }

    let ranges = request
        .ranges
        .iter()
        .map(|item| {
            read_range(&ReadRangeRequest {
                path: request.path.clone(),
                sheet: item.sheet.clone(),
                range: item.range,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let result = ReadRangesResult {
        path: request.path.clone(),
        ranges,
        total_cells,
    };
    Ok(result)
}

pub fn find_cells(request: &FindCellsRequest) -> Result<FindCellsResult, SpreadsheetError> {
    if request.query.is_empty() {
        return Err(SpreadsheetError::InvalidQuery {
            reason: "query must not be empty".to_string(),
        });
    }
    if request.max_results == 0 || request.max_results > MAX_FIND_RESULTS {
        return Err(SpreadsheetError::InvalidQuery {
            reason: format!("maxResults must be between 1 and {MAX_FIND_RESULTS}"),
        });
    }
    if request.range.is_some() && request.sheet.is_none() {
        return Err(SpreadsheetError::InvalidQuery {
            reason: "range requires a sheet".to_string(),
        });
    }
    if let Some(range) = request.range {
        validate_read_range(range)?;
    }

    let (mut workbook, _) = open_workbook_reader(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    if let Some(sheet) = request.sheet.as_deref() {
        if !metadata.iter().any(|candidate| candidate.name == sheet) {
            return Err(SpreadsheetError::SheetNotFound {
                sheet: sheet.to_string(),
            });
        }
    }

    let mut matches = Vec::new();
    let mut scanned_cells = 0usize;
    let mut truncated = false;
    for sheet in metadata {
        let info = sheet_info(&sheet);
        if info.kind != SheetKind::Worksheet
            || request
                .sheet
                .as_deref()
                .is_some_and(|requested| requested != info.name)
        {
            continue;
        }
        let values = worksheet_values(&mut workbook, &request.path, &info.name)?;
        let formulas = worksheet_formulas(&mut workbook, &request.path, &info.name)?;
        let mut positions = HashSet::new();
        add_used_positions(&values, &mut positions, &info.name)?;
        add_used_positions(&formulas, &mut positions, &info.name)?;
        let mut positions = positions.into_iter().collect::<Vec<_>>();
        positions.sort_unstable();

        for (row, column) in positions {
            if request.range.is_some_and(|range| {
                row < range.start.row
                    || row > range.end.row
                    || column < range.start.column
                    || column > range.end.column
            }) {
                continue;
            }
            scanned_cells = scanned_cells.saturating_add(1);
            ensure_workbook_cell_count(scanned_cells)?;
            let value = values.get_value((row, column)).unwrap_or(&Data::Empty);
            let formula = formulas
                .get_value((row, column))
                .filter(|formula| !formula.is_empty())
                .cloned();
            let cell = SpreadsheetCell {
                value: cell_value_from_data(value, &info.name, row, column)?,
                formula,
            };
            let value_text = spreadsheet_cell_display_text(&cell.value);
            let value_matches = text_matches(
                &value_text,
                &request.query,
                request.match_mode,
                request.case_sensitive,
            );
            let formula_matches = request.include_formulas
                && cell.formula.as_deref().is_some_and(|formula| {
                    text_matches(
                        formula,
                        &request.query,
                        request.match_mode,
                        request.case_sensitive,
                    )
                });
            if value_matches || formula_matches {
                if matches.len() == request.max_results {
                    truncated = true;
                    break;
                }
                matches.push(SpreadsheetCellMatch {
                    sheet: info.name.clone(),
                    address: CellAddress { row, column },
                    cell,
                    matched_formula: !value_matches && formula_matches,
                });
            }
        }
        if truncated {
            break;
        }
    }

    let result = FindCellsResult {
        path: request.path.clone(),
        matches,
        scanned_cells: scanned_cells as u64,
        truncated,
    };
    Ok(result)
}

pub fn filter_rows(request: &FilterRowsRequest) -> Result<FilterRowsResult, SpreadsheetError> {
    validate_address(request.range.start)?;
    validate_address(request.range.end)?;
    let columns = request
        .range
        .column_count()
        .ok_or(SpreadsheetError::InvalidRange {
            reason: "range end must not precede range start",
        })?;
    request
        .range
        .row_count()
        .ok_or(SpreadsheetError::InvalidRange {
            reason: "range end must not precede range start",
        })?;
    if request.conditions.is_empty() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "conditions must not be empty".to_string(),
        });
    }
    if request.conditions.len() > MAX_FILTER_CONDITIONS {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("conditions are limited to {MAX_FILTER_CONDITIONS}"),
        });
    }
    if request.max_results == 0 || request.max_results > MAX_FILTER_RESULTS {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("maxResults must be between 1 and {MAX_FILTER_RESULTS}"),
        });
    }
    for condition in &request.conditions {
        validate_filter_condition(condition, request.range)?;
    }
    let returned_cells = columns.saturating_mul(request.max_results as u64);
    if returned_cells > MAX_WORKBOOK_CELLS as u64 {
        return Err(SpreadsheetError::TooManyCells {
            context: "filtered result",
            actual: usize::try_from(returned_cells).unwrap_or(usize::MAX),
            limit: MAX_WORKBOOK_CELLS,
        });
    }

    let (mut workbook, _) = open_workbook_reader(&request.path)?;
    let metadata = workbook.sheets_metadata().to_vec();
    ensure_sheet_count(metadata.len())?;
    let sheet = metadata
        .iter()
        .find(|sheet| sheet.name == request.sheet)
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.sheet.clone(),
        })?;
    let info = sheet_info(sheet);
    if info.kind != SheetKind::Worksheet {
        return Err(SpreadsheetError::UnsupportedSheetType {
            sheet: info.name,
            kind: info.kind,
        });
    }
    let values = worksheet_values(&mut workbook, &request.path, &request.sheet)?;
    let formulas = worksheet_formulas(&mut workbook, &request.path, &request.sheet)?;
    let stats = collect_sheet_stats(&values, &formulas, &request.sheet)?;
    ensure_workbook_cell_count(stats.populated_cells)?;
    let sheet_end_row = stats
        .used_range
        .map(|range| range.end.row)
        .unwrap_or(request.range.start.row.saturating_sub(1));
    let scan_end_row = request.range.end.row.min(sheet_end_row);

    let mut rows = Vec::new();
    let mut matched_row_indices = Vec::new();
    let mut truncated = false;
    let mut scanned_rows = 0_u64;
    for row_index in request.range.start.row..=scan_end_row {
        scanned_rows = scanned_rows.saturating_add(1);
        let condition_matches = request
            .conditions
            .iter()
            .map(|condition| {
                filter_cell(
                    &values,
                    &formulas,
                    &request.sheet,
                    row_index,
                    condition.column,
                )
                .map(|cell| filter_condition_matches(&cell, condition))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let include = match request.match_mode {
            SpreadsheetFilterMatchMode::All => condition_matches.into_iter().all(|matched| matched),
            SpreadsheetFilterMatchMode::Any => condition_matches.into_iter().any(|matched| matched),
        };
        if include {
            if rows.len() == request.max_results {
                truncated = true;
                break;
            }
            let row = (request.range.start.column..=request.range.end.column)
                .map(|column| filter_cell(&values, &formulas, &request.sheet, row_index, column))
                .collect::<Result<Vec<_>, _>>()?;
            matched_row_indices.push(row_index);
            rows.push(row);
        }
    }

    let result = FilterRowsResult {
        path: request.path.clone(),
        sheet: request.sheet.clone(),
        range: request.range,
        matched_row_indices,
        rows,
        scanned_rows,
        truncated,
    };
    Ok(result)
}

fn filter_cell(
    values: &calamine::Range<Data>,
    formulas: &calamine::Range<String>,
    sheet: &str,
    row: u32,
    column: u32,
) -> Result<SpreadsheetCell, SpreadsheetError> {
    let value = values.get_value((row, column)).unwrap_or(&Data::Empty);
    let formula = formulas
        .get_value((row, column))
        .filter(|formula| !formula.is_empty())
        .cloned();
    if let Some(formula) = &formula {
        validate_return_text(formula, sheet, row, column)?;
    }
    Ok(SpreadsheetCell {
        value: cell_value_from_data(value, sheet, row, column)?,
        formula,
    })
}

fn spreadsheet_cell_display_text(value: &SpreadsheetCellValue) -> String {
    match value {
        SpreadsheetCellValue::Empty => String::new(),
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => value.clone(),
        SpreadsheetCellValue::Integer(value) => value.to_string(),
        SpreadsheetCellValue::Number(value) => value.to_string(),
        SpreadsheetCellValue::Boolean(value) => value.to_string(),
        SpreadsheetCellValue::DateTime(value) => value.serial.to_string(),
    }
}

fn text_matches(
    candidate: &str,
    query: &str,
    mode: SpreadsheetTextMatchMode,
    case_sensitive: bool,
) -> bool {
    let (candidate, query) = if case_sensitive {
        (candidate.to_string(), query.to_string())
    } else {
        (candidate.to_lowercase(), query.to_lowercase())
    };
    match mode {
        SpreadsheetTextMatchMode::Contains => candidate.contains(&query),
        SpreadsheetTextMatchMode::Exact => candidate == query,
        SpreadsheetTextMatchMode::StartsWith => candidate.starts_with(&query),
        SpreadsheetTextMatchMode::EndsWith => candidate.ends_with(&query),
    }
}

pub(super) fn validate_filter_condition(
    condition: &SpreadsheetFilterCondition,
    range: CellRange,
) -> Result<(), SpreadsheetError> {
    validate_address(CellAddress {
        row: range.start.row,
        column: condition.column,
    })?;
    if condition.column < range.start.column || condition.column > range.end.column {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!(
                "condition column {} is outside the requested range {}..={}",
                condition.column, range.start.column, range.end.column
            ),
        });
    }
    let blank_operator = matches!(
        condition.operator,
        SpreadsheetFilterOperator::IsBlank | SpreadsheetFilterOperator::IsNotBlank
    );
    if blank_operator && condition.value.is_some() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "is_blank and is_not_blank conditions must omit value".to_string(),
        });
    }
    if !blank_operator && condition.value.is_none() {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("{:?} condition requires value", condition.operator),
        });
    }
    if matches!(
        condition.operator,
        SpreadsheetFilterOperator::GreaterThan
            | SpreadsheetFilterOperator::GreaterThanOrEqual
            | SpreadsheetFilterOperator::LessThan
            | SpreadsheetFilterOperator::LessThanOrEqual
    ) && !matches!(
        condition.value,
        Some(SpreadsheetFilterValue::Integer(_) | SpreadsheetFilterValue::Number(_))
    ) {
        return Err(SpreadsheetError::InvalidFilter {
            reason: "numeric comparison conditions require an integer or number value".to_string(),
        });
    }
    if let Some(SpreadsheetFilterValue::Number(value)) = condition.value {
        if !value.is_finite() {
            return Err(SpreadsheetError::InvalidFilter {
                reason: "filter number must be finite".to_string(),
            });
        }
    }
    Ok(())
}

pub(super) fn filter_condition_matches(
    cell: &SpreadsheetCell,
    condition: &SpreadsheetFilterCondition,
) -> bool {
    match condition.operator {
        SpreadsheetFilterOperator::IsBlank => {
            matches!(cell.value, SpreadsheetCellValue::Empty)
                || spreadsheet_cell_display_text(&cell.value).is_empty()
        }
        SpreadsheetFilterOperator::IsNotBlank => {
            !matches!(cell.value, SpreadsheetCellValue::Empty)
                && !spreadsheet_cell_display_text(&cell.value).is_empty()
        }
        SpreadsheetFilterOperator::Equals => condition
            .value
            .as_ref()
            .is_some_and(|value| filter_values_equal(&cell.value, value, condition.case_sensitive)),
        SpreadsheetFilterOperator::NotEquals => condition.value.as_ref().is_some_and(|value| {
            !filter_values_equal(&cell.value, value, condition.case_sensitive)
        }),
        SpreadsheetFilterOperator::Contains
        | SpreadsheetFilterOperator::NotContains
        | SpreadsheetFilterOperator::StartsWith
        | SpreadsheetFilterOperator::EndsWith => {
            let Some(value) = condition.value.as_ref() else {
                return false;
            };
            let mode = match condition.operator {
                SpreadsheetFilterOperator::Contains => SpreadsheetTextMatchMode::Contains,
                SpreadsheetFilterOperator::NotContains => SpreadsheetTextMatchMode::Contains,
                SpreadsheetFilterOperator::StartsWith => SpreadsheetTextMatchMode::StartsWith,
                SpreadsheetFilterOperator::EndsWith => SpreadsheetTextMatchMode::EndsWith,
                _ => unreachable!(),
            };
            let matched = text_matches(
                &spreadsheet_cell_display_text(&cell.value),
                &filter_value_display_text(value),
                mode,
                condition.case_sensitive,
            );
            if condition.operator == SpreadsheetFilterOperator::NotContains {
                !matched
            } else {
                matched
            }
        }
        SpreadsheetFilterOperator::GreaterThan
        | SpreadsheetFilterOperator::GreaterThanOrEqual
        | SpreadsheetFilterOperator::LessThan
        | SpreadsheetFilterOperator::LessThanOrEqual => {
            let Some(left) = spreadsheet_cell_number(&cell.value) else {
                return false;
            };
            let Some(right) = condition.value.as_ref().and_then(filter_value_number) else {
                return false;
            };
            match condition.operator {
                SpreadsheetFilterOperator::GreaterThan => left > right,
                SpreadsheetFilterOperator::GreaterThanOrEqual => left >= right,
                SpreadsheetFilterOperator::LessThan => left < right,
                SpreadsheetFilterOperator::LessThanOrEqual => left <= right,
                _ => unreachable!(),
            }
        }
    }
}

fn filter_values_equal(
    cell: &SpreadsheetCellValue,
    expected: &SpreadsheetFilterValue,
    case_sensitive: bool,
) -> bool {
    match expected {
        SpreadsheetFilterValue::String(expected) => text_matches(
            &spreadsheet_cell_display_text(cell),
            expected,
            SpreadsheetTextMatchMode::Exact,
            case_sensitive,
        ),
        SpreadsheetFilterValue::Integer(expected) => {
            spreadsheet_cell_number(cell) == Some(*expected as f64)
        }
        SpreadsheetFilterValue::Number(expected) => {
            spreadsheet_cell_number(cell) == Some(*expected)
        }
        SpreadsheetFilterValue::Boolean(expected) => {
            matches!(cell, SpreadsheetCellValue::Boolean(actual) if actual == expected)
        }
    }
}

fn spreadsheet_cell_number(value: &SpreadsheetCellValue) -> Option<f64> {
    match value {
        SpreadsheetCellValue::Integer(value) => Some(*value as f64),
        SpreadsheetCellValue::Number(value) => Some(*value),
        SpreadsheetCellValue::DateTime(value) => Some(value.serial),
        _ => None,
    }
}

fn filter_value_number(value: &SpreadsheetFilterValue) -> Option<f64> {
    match value {
        SpreadsheetFilterValue::Integer(value) => Some(*value as f64),
        SpreadsheetFilterValue::Number(value) => Some(*value),
        _ => None,
    }
}

fn filter_value_display_text(value: &SpreadsheetFilterValue) -> String {
    match value {
        SpreadsheetFilterValue::String(value) => value.clone(),
        SpreadsheetFilterValue::Integer(value) => value.to_string(),
        SpreadsheetFilterValue::Number(value) => value.to_string(),
        SpreadsheetFilterValue::Boolean(value) => value.to_string(),
    }
}
