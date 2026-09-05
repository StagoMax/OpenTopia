use super::{SpreadsheetCellInput, SpreadsheetError};
use chrono::{NaiveDate, NaiveDateTime};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "snake_case",
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
        input_format: String,
        /// Excel-invariant display format applied to every converted cell.
        /// For example, "yyyy-mm-dd" or "yyyy-mm-dd hh:mm:ss".
        output_number_format: String,
    },
    ExtractCurrencyCode,
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
            SpreadsheetValueTransform::ParseDateTime { input_format, .. } => {
                parse_datetime_input(&value, input_format).ok_or_else(|| {
                    invalid_transform(
                        row,
                        column,
                        format!(
                            "could not parse {:?} with format {input_format:?}",
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

pub(crate) fn transform_number_format(
    transforms: &[SpreadsheetValueTransform],
) -> Result<Option<String>, SpreadsheetError> {
    let mut number_format = None;
    for transform in transforms {
        let SpreadsheetValueTransform::ParseDateTime {
            output_number_format,
            ..
        } = transform
        else {
            continue;
        };
        let output_number_format = output_number_format.trim();
        if output_number_format.is_empty()
            || output_number_format.len() > 255
            || output_number_format.contains('\0')
        {
            return Err(SpreadsheetError::InvalidMapping {
                message: "parse_date_time output_number_format must contain 1 to 255 characters"
                    .to_string(),
            });
        }
        if number_format
            .as_deref()
            .is_some_and(|existing| existing != output_number_format)
        {
            return Err(SpreadsheetError::InvalidMapping {
                message: "one conversion cannot request multiple output number formats".to_string(),
            });
        }
        number_format = Some(output_number_format.to_string());
    }
    Ok(number_format)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn converts_currency_and_datetime_values() {
        assert_eq!(
            transform_cell_input(
                SpreadsheetCellInput::String("USD 1,234.50".to_string()),
                &[SpreadsheetValueTransform::ParseNumber { extract: true }],
                1,
                2,
            )
            .unwrap(),
            SpreadsheetCellInput::Number(1234.5)
        );
        assert!(matches!(
            transform_cell_input(
                SpreadsheetCellInput::String("08/19/2026 02:37:00 PM".to_string()),
                &[SpreadsheetValueTransform::ParseDateTime {
                    input_format: "%m/%d/%Y %I:%M:%S %p".to_string(),
                    output_number_format: "yyyy-mm-dd".to_string(),
                }],
                1,
                1,
            )
            .unwrap(),
            SpreadsheetCellInput::Number(_)
        ));
    }
}
