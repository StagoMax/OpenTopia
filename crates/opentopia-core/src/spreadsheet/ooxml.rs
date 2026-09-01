use super::{fs, Cursor, Path, PathBuf, Read, SpreadsheetError, XmlEvent, XmlReader, ZipArchive};
use quick_xml::events::BytesStart;
use std::collections::BTreeMap;

pub(super) type OoxmlArchive = ZipArchive<Cursor<Vec<u8>>>;

pub(super) fn open_ooxml_package(source: &Path) -> Result<OoxmlArchive, SpreadsheetError> {
    let source_bytes = fs::read(source).map_err(|source_error| SpreadsheetError::Io {
        operation: "read",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    ZipArchive::new(Cursor::new(source_bytes)).map_err(|error| SpreadsheetError::InvalidWorkbook {
        path: source.to_path_buf(),
        message: format!("invalid XLSX package: {error}"),
    })
}

pub(super) fn read_zip_part(
    archive: &mut OoxmlArchive,
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

pub(super) fn workbook_sheet_relationships(
    xml: &[u8],
    source: &Path,
) -> Result<Vec<(String, String)>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut relationships = Vec::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "workbook.xml", error))?
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

pub(super) fn workbook_relationship_targets(
    xml: &[u8],
    source: &Path,
) -> Result<BTreeMap<String, String>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut targets = BTreeMap::new();
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "workbook.xml.rels", error))?
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

pub(super) fn xml_attribute(
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

pub(super) fn normalize_workbook_part_target(target: &str) -> String {
    let target = target.replace('\\', "/");
    let target = target.trim_start_matches('/');
    if target.starts_with("xl/") {
        target.to_string()
    } else {
        format!("xl/{}", target.trim_start_matches("../"))
    }
}

pub(super) fn invalid_ooxml_xml(
    source: &Path,
    part: &str,
    error: impl std::fmt::Display,
) -> SpreadsheetError {
    SpreadsheetError::InvalidWorkbook {
        path: source.to_path_buf(),
        message: format!("invalid {part}: {error}"),
    }
}
