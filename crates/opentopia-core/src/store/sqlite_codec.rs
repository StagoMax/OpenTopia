use crate::model::{ArtifactStorage, ArtifactStorageMetadata};
use chrono::{DateTime, Utc};
use rusqlite::types::Type;
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

pub(super) fn parse_uuid(value: String, column: usize) -> rusqlite::Result<Uuid> {
    Uuid::parse_str(&value)
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

pub(super) fn deserialize_json_column<T>(row: &rusqlite::Row<'_>) -> rusqlite::Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let document: String = row.get(0)?;
    serde_json::from_str(&document)
        .map_err(|error| rusqlite::Error::FromSqlConversionFailure(0, Type::Text, Box::new(error)))
}

pub(super) fn collect_rows<T, F>(rows: rusqlite::MappedRows<'_, F>) -> anyhow::Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut output = Vec::new();
    for row in rows {
        output.push(row?);
    }
    Ok(output)
}

pub(super) fn parse_artifact_storage_metadata(
    storage_kind: &str,
    path: Option<String>,
    column: usize,
) -> rusqlite::Result<ArtifactStorageMetadata> {
    match storage_kind {
        "inline" => Ok(ArtifactStorageMetadata::Inline),
        "path" => path
            .map(|path| ArtifactStorageMetadata::Path {
                path: PathBuf::from(path),
            })
            .ok_or_else(|| invalid_column(column, "artifact path storage missing path")),
        other => Err(invalid_column(
            column,
            format!("unknown artifact storage kind: {other}"),
        )),
    }
}

pub(super) fn parse_artifact_storage(
    storage_kind: &str,
    path: Option<String>,
    inline_content: Option<String>,
    column: usize,
) -> rusqlite::Result<ArtifactStorage> {
    match storage_kind {
        "inline" => inline_content
            .map(|content| ArtifactStorage::Inline { content })
            .ok_or_else(|| invalid_column(column, "inline artifact missing content")),
        "path" => path
            .map(|path| ArtifactStorage::Path {
                path: PathBuf::from(path),
            })
            .ok_or_else(|| invalid_column(column, "path artifact missing path")),
        other => Err(invalid_column(
            column,
            format!("unknown artifact storage kind: {other}"),
        )),
    }
}

pub(super) fn parse_u64(value: i64, column: usize) -> rusqlite::Result<u64> {
    u64::try_from(value).map_err(|err| {
        rusqlite::Error::FromSqlConversionFailure(column, Type::Integer, Box::new(err))
    })
}

pub(super) fn parse_datetime(value: String, column: usize) -> rusqlite::Result<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(&value)
        .map(|value| value.with_timezone(&Utc))
        .map_err(|err| rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(err)))
}

pub(super) fn invalid_column(column: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        column,
        Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}
