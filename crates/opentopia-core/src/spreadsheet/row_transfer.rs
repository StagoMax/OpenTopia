use super::transfer::{headers_from_sheet, resolve_selector, stored_cell_value};
use super::{
    apply_sheet_updates, ensure_workbook_cell_count, load_spreadsheet, load_workbook,
    patch_workbook_template, validate_address, validate_return_text, CellAddress, CellRange,
    DelimitedHeader, SheetWriteRequest, SpreadsheetCell, SpreadsheetCellInput,
    SpreadsheetCellValue, SpreadsheetColumnSelector, SpreadsheetError, SpreadsheetFileFormat,
    SpreadsheetFilterCondition, SpreadsheetFilterMatchMode, SpreadsheetFilterOperator,
    SpreadsheetFilterValue, EXCEL_MAX_COLUMNS, MAX_FILTER_CONDITIONS, MAX_OUTPUT_FILE_BYTES,
    MAX_WORKBOOK_CELLS,
};
use chrono::{NaiveDate, NaiveDateTime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferRowFilter {
    pub source: SpreadsheetColumnSelector,
    pub operator: SpreadsheetFilterOperator,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<SpreadsheetFilterValue>,
    #[serde(default)]
    pub case_sensitive: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SpreadsheetValueTransform {
    AsString,
    Trim,
    ParseNumber {
        /// Extract the first signed decimal from surrounding text such as "USD 12.50".
        #[serde(default)]
        extract: bool,
    },
    ParseDateTime {
        /// Chrono/strftime input format, for example "%m/%d/%Y %I:%M:%S %p".
        format: String,
    },
    ExtractCurrencyCode,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum TransferColumnValue {
    Source {
        source: SpreadsheetColumnSelector,
        #[serde(default)]
        transforms: Vec<SpreadsheetValueTransform>,
    },
    Constant {
        value: SpreadsheetCellInput,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferColumn {
    pub target: SpreadsheetColumnSelector,
    pub value: TransferColumnValue,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransferRowsRequest {
    pub source: PathBuf,
    pub source_sheet: String,
    pub template: PathBuf,
    pub output: PathBuf,
    pub target_sheet: String,
    #[serde(default)]
    pub source_header_row: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_start_row: Option<u32>,
    #[serde(default)]
    pub target_header_row: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_start_row: Option<u32>,
    #[serde(default)]
    #[schemars(length(max = 32))]
    pub filters: Vec<TransferRowFilter>,
    #[serde(default)]
    pub filter_match_mode: SpreadsheetFilterMatchMode,
    #[schemars(length(min = 1, max = 256))]
    pub columns: Vec<TransferColumn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedTransferColumn {
    pub target_column: u32,
    pub target_header: Option<String>,
    pub source_column: Option<u32>,
    pub source_header: Option<String>,
    pub constant: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TransferRowsResult {
    pub source: PathBuf,
    pub template: PathBuf,
    pub output: PathBuf,
    pub source_format: String,
    pub rows_scanned: u32,
    pub rows_matched: u32,
    pub rows_written: u32,
    pub rows_skipped: u32,
    pub columns_written: usize,
    pub cells_written: usize,
    pub bytes_written: u64,
    pub target_range: Option<CellRange>,
    pub columns: Vec<ResolvedTransferColumn>,
    pub validation_reopened: bool,
}

struct ResolvedColumn<'a> {
    target: DelimitedHeader,
    source: Option<DelimitedHeader>,
    transforms: &'a [SpreadsheetValueTransform],
    constant: Option<&'a SpreadsheetCellInput>,
}

pub(super) fn transfer_rows(
    request: &TransferRowsRequest,
) -> Result<TransferRowsResult, SpreadsheetError> {
    let source_format = SpreadsheetFileFormat::from_path(&request.source).ok_or_else(|| {
        SpreadsheetError::UnsupportedFormat {
            path: request.source.clone(),
            extension: request
                .source
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_string),
        }
    })?;
    super::validate_xlsx_path(&request.template)?;
    super::validate_xlsx_path(&request.output)?;
    if request.filters.len() > MAX_FILTER_CONDITIONS {
        return Err(SpreadsheetError::InvalidFilter {
            reason: format!("conditions are limited to {MAX_FILTER_CONDITIONS}"),
        });
    }
    if request.columns.is_empty() {
        return Err(SpreadsheetError::InvalidMapping {
            message: "columns must not be empty".to_string(),
        });
    }

    let source_workbook = load_spreadsheet(&request.source)?;
    let source_sheet = source_workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(request.source_sheet.trim()))
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.source_sheet.clone(),
        })?;
    let source_headers = headers_from_sheet(source_sheet, request.source_header_row);
    let source_column_count = source_sheet
        .cells
        .keys()
        .map(|address| address.column)
        .max()
        .map(|column| column.saturating_add(1))
        .unwrap_or(0);
    if source_column_count == 0 {
        return Err(SpreadsheetError::InvalidMapping {
            message: "source sheet has no columns".to_string(),
        });
    }
    let source_start_row = request
        .source_start_row
        .unwrap_or_else(|| request.source_header_row.saturating_add(1));
    if source_start_row <= request.source_header_row {
        return Err(SpreadsheetError::InvalidMapping {
            message: "sourceStartRow must be after sourceHeaderRow".to_string(),
        });
    }

    let mut target_workbook = load_workbook(&request.template)?;
    let target_index = target_workbook
        .sheets
        .iter()
        .position(|sheet| sheet.name.eq_ignore_ascii_case(request.target_sheet.trim()))
        .ok_or_else(|| SpreadsheetError::SheetNotFound {
            sheet: request.target_sheet.clone(),
        })?;
    let target_sheet = &target_workbook.sheets[target_index];
    let target_headers = headers_from_sheet(target_sheet, request.target_header_row);
    let target_start_row = request
        .target_start_row
        .unwrap_or_else(|| request.target_header_row.saturating_add(1));
    if target_start_row <= request.target_header_row {
        return Err(SpreadsheetError::InvalidMapping {
            message: "targetStartRow must be after targetHeaderRow".to_string(),
        });
    }

    let resolved_filters = resolve_filters(&request.filters, &source_headers, source_column_count)?;
    let resolved_columns = resolve_columns(
        &request.columns,
        &source_headers,
        source_column_count,
        &target_headers,
    )?;
    let source_end_row = source_sheet
        .cells
        .keys()
        .map(|address| address.row)
        .max()
        .unwrap_or(source_start_row.saturating_sub(1));

    let mut updates = Vec::new();
    let mut rows_scanned = 0_u32;
    let mut rows_written = 0_u32;
    for source_row in source_start_row..=source_end_row {
        let row_start = CellAddress {
            row: source_row,
            column: 0,
        };
        let row_end = CellAddress {
            row: source_row,
            column: EXCEL_MAX_COLUMNS - 1,
        };
        if source_sheet
            .cells
            .range(row_start..=row_end)
            .next()
            .is_none()
        {
            continue;
        }
        rows_scanned = rows_scanned.saturating_add(1);
        let matches = resolved_filters.is_empty()
            || match request.filter_match_mode {
                SpreadsheetFilterMatchMode::All => resolved_filters.iter().all(|condition| {
                    let cell = source_cell(source_sheet, source_row, condition.column);
                    super::read::filter_condition_matches(&cell, condition)
                }),
                SpreadsheetFilterMatchMode::Any => resolved_filters.iter().any(|condition| {
                    let cell = source_cell(source_sheet, source_row, condition.column);
                    super::read::filter_condition_matches(&cell, condition)
                }),
            };
        if !matches {
            continue;
        }

        let target_row = target_start_row.checked_add(rows_written).ok_or_else(|| {
            SpreadsheetError::InvalidMapping {
                message: "target row overflow".to_string(),
            }
        })?;
        for column in &resolved_columns {
            let address = CellAddress {
                row: target_row,
                column: column.target.column,
            };
            validate_address(address)?;
            let value = if let Some(constant) = column.constant {
                constant.clone()
            } else {
                let source = column.source.as_ref().expect("resolved source column");
                let cell = source_cell(source_sheet, source_row, source.column);
                transform_cell(cell.value, column.transforms, source_row, source.column)?
            };
            if let SpreadsheetCellInput::String(value) = &value {
                validate_return_text(
                    value,
                    &request.target_sheet,
                    target_row,
                    column.target.column,
                )?;
            }
            updates.push(super::CellUpdate { address, value });
            if updates.len() > MAX_WORKBOOK_CELLS {
                return Err(SpreadsheetError::TooManyCells {
                    context: "row transfer",
                    actual: updates.len(),
                    limit: MAX_WORKBOOK_CELLS,
                });
            }
        }
        rows_written = rows_written.saturating_add(1);
    }

    let sheet_request = SheetWriteRequest {
        name: target_workbook.sheets[target_index].name.clone(),
        visibility: None,
        cells: updates,
    };
    apply_sheet_updates(&mut target_workbook, std::slice::from_ref(&sheet_request))?;
    let output_cells = target_workbook
        .sheets
        .iter()
        .map(|sheet| sheet.cells.len())
        .sum::<usize>();
    ensure_workbook_cell_count(output_cells)?;

    let bytes = patch_workbook_template(
        &request.template,
        std::slice::from_ref(&sheet_request),
        &request.output,
    )?;
    let bytes_written = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_written > MAX_OUTPUT_FILE_BYTES {
        return Err(SpreadsheetError::OutputTooLarge {
            actual_bytes: bytes_written,
            limit_bytes: MAX_OUTPUT_FILE_BYTES,
        });
    }
    fs::write(&request.output, &bytes).map_err(|source| SpreadsheetError::Io {
        operation: "write",
        path: request.output.clone(),
        source,
    })?;
    verify_output(&request.output, &sheet_request)?;

    let target_range = resolved_columns
        .first()
        .filter(|_| rows_written > 0)
        .map(|_| CellRange {
            start: CellAddress {
                row: target_start_row,
                column: resolved_columns
                    .iter()
                    .map(|column| column.target.column)
                    .min()
                    .expect("columns exist"),
            },
            end: CellAddress {
                row: target_start_row + rows_written - 1,
                column: resolved_columns
                    .iter()
                    .map(|column| column.target.column)
                    .max()
                    .expect("columns exist"),
            },
        });
    let columns = resolved_columns
        .iter()
        .map(|column| ResolvedTransferColumn {
            target_column: column.target.column,
            target_header: Some(column.target.name.clone()).filter(|name| !name.is_empty()),
            source_column: column.source.as_ref().map(|source| source.column),
            source_header: column
                .source
                .as_ref()
                .map(|source| source.name.clone())
                .filter(|name| !name.is_empty()),
            constant: column.constant.is_some(),
        })
        .collect();
    Ok(TransferRowsResult {
        source: request.source.clone(),
        template: request.template.clone(),
        output: request.output.clone(),
        source_format: source_format.extension().to_string(),
        rows_scanned,
        rows_matched: rows_written,
        rows_written,
        rows_skipped: rows_scanned.saturating_sub(rows_written),
        columns_written: resolved_columns.len(),
        cells_written: sheet_request.cells.len(),
        bytes_written,
        target_range,
        columns,
        validation_reopened: true,
    })
}

fn resolve_filters(
    filters: &[TransferRowFilter],
    headers: &[DelimitedHeader],
    column_count: u32,
) -> Result<Vec<SpreadsheetFilterCondition>, SpreadsheetError> {
    filters
        .iter()
        .map(|filter| {
            let source = resolve_selector(&filter.source, headers, column_count, "source")?;
            let condition = SpreadsheetFilterCondition {
                column: source.column,
                operator: filter.operator,
                value: filter.value.clone(),
                case_sensitive: filter.case_sensitive,
            };
            super::read::validate_filter_condition(
                &condition,
                CellRange {
                    start: CellAddress { row: 0, column: 0 },
                    end: CellAddress {
                        row: 0,
                        column: column_count - 1,
                    },
                },
            )?;
            Ok(condition)
        })
        .collect()
}

fn resolve_columns<'a>(
    columns: &'a [TransferColumn],
    source_headers: &[DelimitedHeader],
    source_column_count: u32,
    target_headers: &[DelimitedHeader],
) -> Result<Vec<ResolvedColumn<'a>>, SpreadsheetError> {
    let mut targets = BTreeSet::new();
    let mut resolved = Vec::with_capacity(columns.len());
    for column in columns {
        let target = resolve_selector(&column.target, target_headers, EXCEL_MAX_COLUMNS, "target")?;
        if !targets.insert(target.column) {
            return Err(SpreadsheetError::InvalidMapping {
                message: format!("target column {} is mapped more than once", target.column),
            });
        }
        let (source, transforms, constant) = match &column.value {
            TransferColumnValue::Source { source, transforms } => (
                Some(resolve_selector(
                    source,
                    source_headers,
                    source_column_count,
                    "source",
                )?),
                transforms.as_slice(),
                None,
            ),
            TransferColumnValue::Constant { value } => (None, &[][..], Some(value)),
        };
        resolved.push(ResolvedColumn {
            target,
            source,
            transforms,
            constant,
        });
    }
    resolved.sort_by_key(|column| column.target.column);
    Ok(resolved)
}

fn source_cell(sheet: &super::LoadedSheet, row: u32, column: u32) -> SpreadsheetCell {
    sheet
        .cells
        .get(&CellAddress { row, column })
        .map(|cell| SpreadsheetCell {
            value: cell.value.clone(),
            formula: cell.formula.clone(),
        })
        .unwrap_or(SpreadsheetCell {
            value: SpreadsheetCellValue::Empty,
            formula: None,
        })
}

pub(crate) fn transform_cell(
    source: SpreadsheetCellValue,
    transforms: &[SpreadsheetValueTransform],
    row: u32,
    column: u32,
) -> Result<SpreadsheetCellInput, SpreadsheetError> {
    transform_cell_input(cell_input(source), transforms, row, column)
}

pub(crate) fn transform_cell_input(
    mut value: SpreadsheetCellInput,
    transforms: &[SpreadsheetValueTransform],
    row: u32,
    column: u32,
) -> Result<SpreadsheetCellInput, SpreadsheetError> {
    for transform in transforms {
        value = match transform {
            SpreadsheetValueTransform::AsString => match value {
                SpreadsheetCellInput::Blank => SpreadsheetCellInput::Blank,
                value => SpreadsheetCellInput::String(input_text(&value)),
            },
            SpreadsheetValueTransform::Trim => match value {
                SpreadsheetCellInput::Blank => SpreadsheetCellInput::Blank,
                value => SpreadsheetCellInput::String(input_text(&value).trim().to_string()),
            },
            SpreadsheetValueTransform::ParseNumber { extract } => {
                parse_number_input(&value, *extract).ok_or_else(|| {
                    invalid_transform(
                        row,
                        column,
                        format!("could not parse {:?} as a number", input_text(&value)),
                    )
                })?
            }
            SpreadsheetValueTransform::ParseDateTime { format } => {
                parse_datetime_input(&value, format).ok_or_else(|| {
                    invalid_transform(
                        row,
                        column,
                        format!(
                            "could not parse {:?} with format {format:?}",
                            input_text(&value)
                        ),
                    )
                })?
            }
            SpreadsheetValueTransform::ExtractCurrencyCode => {
                let text = input_text(&value);
                let currency = text
                    .split_whitespace()
                    .next()
                    .unwrap_or_default()
                    .trim_matches(|character: char| !character.is_ascii_alphanumeric());
                if currency.is_empty() {
                    return Err(invalid_transform(
                        row,
                        column,
                        format!("could not extract a currency code from {text:?}"),
                    ));
                }
                SpreadsheetCellInput::String(currency.to_string())
            }
        };
    }
    Ok(value)
}

fn cell_input(value: SpreadsheetCellValue) -> SpreadsheetCellInput {
    match value {
        SpreadsheetCellValue::Empty => SpreadsheetCellInput::Blank,
        SpreadsheetCellValue::String(value)
        | SpreadsheetCellValue::DateTimeIso(value)
        | SpreadsheetCellValue::DurationIso(value)
        | SpreadsheetCellValue::Error(value) => SpreadsheetCellInput::String(value),
        SpreadsheetCellValue::Integer(value) => SpreadsheetCellInput::Integer(value),
        SpreadsheetCellValue::Number(value) => SpreadsheetCellInput::Number(value),
        SpreadsheetCellValue::Boolean(value) => SpreadsheetCellInput::Boolean(value),
        SpreadsheetCellValue::DateTime(value) => SpreadsheetCellInput::Number(value.serial),
    }
}

fn input_text(value: &SpreadsheetCellInput) -> String {
    match value {
        SpreadsheetCellInput::Blank => String::new(),
        SpreadsheetCellInput::String(value) => value.clone(),
        SpreadsheetCellInput::Integer(value) => value.to_string(),
        SpreadsheetCellInput::Number(value) => value.to_string(),
        SpreadsheetCellInput::Boolean(value) => value.to_string(),
        SpreadsheetCellInput::Formula(value) => value.expression.clone(),
    }
}

fn parse_number_input(value: &SpreadsheetCellInput, extract: bool) -> Option<SpreadsheetCellInput> {
    match value {
        SpreadsheetCellInput::Integer(value) => Some(SpreadsheetCellInput::Integer(*value)),
        SpreadsheetCellInput::Number(value) => Some(SpreadsheetCellInput::Number(*value)),
        _ => {
            let normalized = input_text(value).replace(',', "");
            let candidate = if extract {
                first_decimal(&normalized)?
            } else {
                normalized.trim()
            };
            let number = candidate
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())?;
            if number.fract() == 0.0 && number >= i64::MIN as f64 && number <= i64::MAX as f64 {
                Some(SpreadsheetCellInput::Integer(number as i64))
            } else {
                Some(SpreadsheetCellInput::Number(number))
            }
        }
    }
}

fn first_decimal(value: &str) -> Option<&str> {
    let bytes = value.as_bytes();
    let mut start = None;
    let mut dot = false;
    for (index, byte) in bytes.iter().copied().enumerate() {
        if start.is_none() {
            let signed = matches!(byte, b'+' | b'-')
                && bytes
                    .get(index + 1)
                    .is_some_and(|next| next.is_ascii_digit() || *next == b'.');
            if byte.is_ascii_digit()
                || signed
                || (byte == b'.' && bytes.get(index + 1).is_some_and(u8::is_ascii_digit))
            {
                start = Some(index);
                dot = byte == b'.';
            }
            continue;
        }
        if byte.is_ascii_digit() {
            continue;
        }
        if byte == b'.' && !dot {
            dot = true;
            continue;
        }
        return start.map(|start| &value[start..index]);
    }
    start.map(|start| &value[start..])
}

fn parse_datetime_input(
    value: &SpreadsheetCellInput,
    format: &str,
) -> Option<SpreadsheetCellInput> {
    if let SpreadsheetCellInput::Number(value) = value {
        return Some(SpreadsheetCellInput::Number(*value));
    }
    let text = input_text(value);
    let datetime = NaiveDateTime::parse_from_str(text.trim(), format)
        .ok()
        .or_else(|| {
            NaiveDate::parse_from_str(text.trim(), format)
                .ok()
                .and_then(|date| date.and_hms_opt(0, 0, 0))
        })?;
    let epoch = NaiveDate::from_ymd_opt(1899, 12, 30)?.and_hms_opt(0, 0, 0)?;
    let milliseconds = datetime.signed_duration_since(epoch).num_milliseconds();
    Some(SpreadsheetCellInput::Number(
        milliseconds as f64 / 86_400_000.0,
    ))
}

fn invalid_transform(row: u32, column: u32, message: String) -> SpreadsheetError {
    SpreadsheetError::InvalidMapping {
        message: format!("source row {row}, column {column}: {message}"),
    }
}

fn verify_output(output: &Path, request: &SheetWriteRequest) -> Result<(), SpreadsheetError> {
    let workbook = load_workbook(output)?;
    let sheet = workbook
        .sheets
        .iter()
        .find(|sheet| sheet.name.eq_ignore_ascii_case(&request.name))
        .ok_or_else(|| SpreadsheetError::ValidationFailed {
            message: format!("output sheet {:?} was not found", request.name),
        })?;
    for update in &request.cells {
        let actual = sheet.cells.get(&update.address);
        if !input_matches(&update.value, actual) {
            return Err(SpreadsheetError::ValidationFailed {
                message: format!(
                    "output did not preserve transferred value at {}!R{}C{}",
                    request.name, update.address.row, update.address.column
                ),
            });
        }
    }
    Ok(())
}

fn input_matches(expected: &SpreadsheetCellInput, actual: Option<&super::StoredCell>) -> bool {
    match (expected, actual) {
        (SpreadsheetCellInput::Blank, None) => true,
        (SpreadsheetCellInput::Blank, Some(cell)) => {
            matches!(cell.value, SpreadsheetCellValue::Empty) && cell.formula.is_none()
        }
        (SpreadsheetCellInput::String(expected), Some(cell)) => {
            cell.formula.is_none() && stored_cell_value(cell) == *expected
        }
        (SpreadsheetCellInput::Integer(expected), Some(cell)) => match cell.value {
            SpreadsheetCellValue::Integer(actual) => actual == *expected,
            SpreadsheetCellValue::Number(actual) => actual == *expected as f64,
            _ => false,
        },
        (SpreadsheetCellInput::Number(expected), Some(cell)) => match cell.value {
            SpreadsheetCellValue::Integer(actual) => actual as f64 == *expected,
            SpreadsheetCellValue::Number(actual) => (actual - expected).abs() < 1e-9,
            SpreadsheetCellValue::DateTime(actual) => (actual.serial - expected).abs() < 1e-9,
            _ => false,
        },
        (SpreadsheetCellInput::Boolean(expected), Some(cell)) => {
            matches!(cell.value, SpreadsheetCellValue::Boolean(actual) if actual == *expected)
        }
        (SpreadsheetCellInput::Formula(expected), Some(cell)) => {
            cell.formula.as_deref() == Some(expected.expression.as_str())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_embedded_numbers_and_converts_excel_dates() {
        assert_eq!(
            first_decimal("USD -1,234.50".replace(',', "").as_str()),
            Some("-1234.50")
        );
        assert_eq!(
            parse_number_input(&SpreadsheetCellInput::String("USD 12.50".to_string()), true),
            Some(SpreadsheetCellInput::Number(12.5))
        );
        assert_eq!(
            parse_datetime_input(
                &SpreadsheetCellInput::String("08/19/2026 02:37:00 AM".to_string()),
                "%m/%d/%Y %I:%M:%S %p",
            ),
            Some(SpreadsheetCellInput::Number(46253.10902777778))
        );
    }
}
