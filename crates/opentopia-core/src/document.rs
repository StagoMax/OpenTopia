use crate::artifact_runtime::{
    ValidationIssue, ValidationReport, ValidationSeverity, MAX_ARTIFACT_INPUT_BYTES,
};
use quick_xml::events::Event;
use quick_xml::name::{Namespace, ResolveResult};
use quick_xml::reader::NsReader;
use quick_xml::Reader;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Cursor, Read};
use std::path::{Component, Path, PathBuf};
use zip::ZipArchive;

pub const MAX_DOCUMENT_EXTRACT_CHARACTERS: usize = 200_000;
const MAX_OPC_PARTS: usize = 4_096;
const MAX_OPC_UNCOMPRESSED_BYTES: u64 = 256 * 1024 * 1024;
const MAX_XML_PART_BYTES: u64 = 32 * 1024 * 1024;
const MAIN_DOCUMENT_PART: &str = "word/document.xml";
const WORDPROCESSINGML_NAMESPACE: &[u8] =
    b"http://schemas.openxmlformats.org/wordprocessingml/2006/main";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentInspection {
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub part_count: usize,
    pub uncompressed_bytes: u64,
    pub paragraph_count: usize,
    pub table_count: usize,
    pub text_characters: usize,
    pub media_count: usize,
    pub embedded_object_count: usize,
    pub comment_part_count: usize,
    pub header_count: usize,
    pub footer_count: usize,
    pub tracked_insertions: usize,
    pub tracked_deletions: usize,
    pub has_macros: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentPartText {
    pub part: String,
    pub text: String,
    pub character_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentExtraction {
    pub path: PathBuf,
    pub parts: Vec<DocumentPartText>,
    pub total_characters: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DocumentValidation {
    pub path: PathBuf,
    pub report: ValidationReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<DocumentInspection>,
}

#[derive(Debug, thiserror::Error)]
pub enum DocumentError {
    #[error("unsupported document path {0}: expected a .docx file")]
    UnsupportedFormat(PathBuf),
    #[error("DOCX input is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    FileTooLarge { actual_bytes: u64, limit_bytes: u64 },
    #[error("invalid DOCX {path}: {message}")]
    InvalidDocument { path: PathBuf, message: String },
    #[error("DOCX part {part} is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    PartTooLarge {
        part: String,
        actual_bytes: u64,
        limit_bytes: u64,
    },
    #[error("maxCharacters must be between 1 and {MAX_DOCUMENT_EXTRACT_CHARACTERS}")]
    InvalidCharacterLimit,
}

pub fn inspect_document(path: &Path, bytes: &[u8]) -> Result<DocumentInspection, DocumentError> {
    validate_document_input(path, bytes)?;
    let mut archive = open_archive(path, bytes)?;
    let package = inspect_package(path, &mut archive)?;
    validate_opc_metadata(path, &mut archive)?;
    let main_xml = read_part(path, &mut archive, MAIN_DOCUMENT_PART)?;
    let parsed = parse_wordprocessing_xml(path, MAIN_DOCUMENT_PART, &main_xml)?;
    Ok(DocumentInspection {
        path: path.to_path_buf(),
        file_size_bytes: bytes.len() as u64,
        part_count: package.part_count,
        uncompressed_bytes: package.uncompressed_bytes,
        paragraph_count: parsed.paragraph_count,
        table_count: parsed.table_count,
        text_characters: parsed.text.chars().count(),
        media_count: package.media_count,
        embedded_object_count: package.embedded_object_count,
        comment_part_count: package.comment_part_count,
        header_count: package.header_count,
        footer_count: package.footer_count,
        tracked_insertions: parsed.tracked_insertions,
        tracked_deletions: parsed.tracked_deletions,
        has_macros: package.has_macros,
    })
}

pub fn extract_document_text(
    path: &Path,
    bytes: &[u8],
    include_related_parts: bool,
    max_characters: usize,
) -> Result<DocumentExtraction, DocumentError> {
    validate_document_input(path, bytes)?;
    if !(1..=MAX_DOCUMENT_EXTRACT_CHARACTERS).contains(&max_characters) {
        return Err(DocumentError::InvalidCharacterLimit);
    }
    let mut archive = open_archive(path, bytes)?;
    let package = inspect_package(path, &mut archive)?;
    let mut part_names = vec![MAIN_DOCUMENT_PART.to_string()];
    if include_related_parts {
        part_names.extend(package.related_text_parts);
    }
    let mut parts = Vec::with_capacity(part_names.len());
    let mut remaining = max_characters;
    let mut total_characters = 0_usize;
    let mut truncated = false;
    for part in part_names {
        if remaining == 0 {
            truncated = true;
            break;
        }
        let xml = read_part(path, &mut archive, &part)?;
        let parsed = parse_wordprocessing_xml(path, &part, &xml)?;
        let character_count = parsed.text.chars().count();
        let part_truncated = character_count > remaining;
        let text = if part_truncated {
            parsed.text.chars().take(remaining).collect()
        } else {
            parsed.text
        };
        let returned = text.chars().count();
        remaining = remaining.saturating_sub(returned);
        total_characters = total_characters.saturating_add(returned);
        truncated |= part_truncated;
        parts.push(DocumentPartText {
            part,
            text,
            character_count,
            truncated: part_truncated,
        });
        if part_truncated {
            break;
        }
    }
    Ok(DocumentExtraction {
        path: path.to_path_buf(),
        parts,
        total_characters,
        truncated,
    })
}

pub fn validate_document(path: &Path, bytes: &[u8]) -> DocumentValidation {
    let mut issues = Vec::new();
    let inspection = match inspect_document(path, bytes) {
        Ok(inspection) => Some(inspection),
        Err(error) => {
            issues.push(issue(
                "invalid_docx",
                ValidationSeverity::Error,
                error.to_string(),
                None,
            ));
            None
        }
    };
    if let Some(inspection) = &inspection {
        if inspection.has_macros {
            issues.push(issue(
                "macros_present",
                ValidationSeverity::Warning,
                "the package contains a VBA project; edits must preserve or explicitly remove it",
                Some("word/vbaProject.bin"),
            ));
        }
        if inspection.embedded_object_count > 0 {
            issues.push(issue(
                "embedded_objects_present",
                ValidationSeverity::Info,
                "the package contains embedded objects that must survive round-trip edits",
                Some("word/embeddings"),
            ));
        }
        if inspection.tracked_insertions > 0 || inspection.tracked_deletions > 0 {
            issues.push(issue(
                "tracked_changes_present",
                ValidationSeverity::Info,
                "the document contains tracked changes",
                Some(MAIN_DOCUMENT_PART),
            ));
        }
    }
    DocumentValidation {
        path: path.to_path_buf(),
        report: ValidationReport::from_issues(issues),
        inspection,
    }
}

fn validate_document_input(path: &Path, bytes: &[u8]) -> Result<(), DocumentError> {
    let is_docx = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("docx"));
    if !is_docx {
        return Err(DocumentError::UnsupportedFormat(path.to_path_buf()));
    }
    if bytes.len() as u64 > MAX_ARTIFACT_INPUT_BYTES {
        return Err(DocumentError::FileTooLarge {
            actual_bytes: bytes.len() as u64,
            limit_bytes: MAX_ARTIFACT_INPUT_BYTES,
        });
    }
    Ok(())
}

type DocumentArchive<'a> = ZipArchive<Cursor<&'a [u8]>>;

fn open_archive<'a>(path: &Path, bytes: &'a [u8]) -> Result<DocumentArchive<'a>, DocumentError> {
    validate_zip_entry_budget(path, bytes)?;
    ZipArchive::new(Cursor::new(bytes)).map_err(|error| DocumentError::InvalidDocument {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn validate_zip_entry_budget(path: &Path, bytes: &[u8]) -> Result<(), DocumentError> {
    const EOCD_SIGNATURE: &[u8; 4] = b"PK\x05\x06";
    const EOCD_SIZE: usize = 22;
    const MAX_ZIP_COMMENT_BYTES: usize = u16::MAX as usize;
    let search_start = bytes
        .len()
        .saturating_sub(EOCD_SIZE + MAX_ZIP_COMMENT_BYTES);
    let tail = &bytes[search_start..];
    let Some(offset) = (0..tail.len().saturating_sub(3)).rev().find(|offset| {
        let offset = *offset;
        if tail.get(offset..offset + 4) != Some(EOCD_SIGNATURE) || offset + EOCD_SIZE > tail.len() {
            return false;
        }
        let comment_length = u16::from_le_bytes([tail[offset + 20], tail[offset + 21]]) as usize;
        offset + EOCD_SIZE + comment_length == tail.len()
    }) else {
        return Err(invalid(
            path,
            "ZIP end-of-central-directory record is missing",
        ));
    };
    let disk_number = u16::from_le_bytes([tail[offset + 4], tail[offset + 5]]);
    let central_directory_disk = u16::from_le_bytes([tail[offset + 6], tail[offset + 7]]);
    let entries_on_disk = u16::from_le_bytes([tail[offset + 8], tail[offset + 9]]);
    let total_entries = u16::from_le_bytes([tail[offset + 10], tail[offset + 11]]);
    if disk_number != 0 || central_directory_disk != 0 || entries_on_disk != total_entries {
        return Err(invalid(path, "multi-disk ZIP packages are not supported"));
    }
    if total_entries == u16::MAX {
        return Err(invalid(
            path,
            "ZIP64 package directories are not supported for DOCX inputs",
        ));
    }
    if usize::from(total_entries) > MAX_OPC_PARTS {
        return Err(invalid(
            path,
            format!("package has {total_entries} parts; limit is {MAX_OPC_PARTS}"),
        ));
    }
    Ok(())
}

#[derive(Default)]
struct PackageInspection {
    part_count: usize,
    uncompressed_bytes: u64,
    media_count: usize,
    embedded_object_count: usize,
    comment_part_count: usize,
    header_count: usize,
    footer_count: usize,
    has_macros: bool,
    related_text_parts: Vec<String>,
}

fn inspect_package(
    path: &Path,
    archive: &mut DocumentArchive<'_>,
) -> Result<PackageInspection, DocumentError> {
    if archive.len() > MAX_OPC_PARTS {
        return Err(invalid(
            path,
            format!(
                "package has {} parts; limit is {MAX_OPC_PARTS}",
                archive.len()
            ),
        ));
    }
    let mut result = PackageInspection::default();
    let mut names = HashSet::new();
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|error| invalid(path, error.to_string()))?;
        let name = file.name().replace('\\', "/");
        if !safe_part_name(&name) {
            return Err(invalid(path, format!("unsafe package part name {name:?}")));
        }
        if !names.insert(name.clone()) {
            return Err(invalid(path, format!("duplicate package part {name:?}")));
        }
        result.part_count += 1;
        result.uncompressed_bytes = result
            .uncompressed_bytes
            .checked_add(file.size())
            .ok_or_else(|| invalid(path, "package size overflow"))?;
        if result.uncompressed_bytes > MAX_OPC_UNCOMPRESSED_BYTES {
            return Err(invalid(
                path,
                format!("uncompressed package exceeds {MAX_OPC_UNCOMPRESSED_BYTES} bytes"),
            ));
        }
        let lower = name.to_ascii_lowercase();
        result.media_count += usize::from(lower.starts_with("word/media/") && !name.ends_with('/'));
        result.embedded_object_count +=
            usize::from(lower.starts_with("word/embeddings/") && !name.ends_with('/'));
        result.comment_part_count +=
            usize::from(lower.starts_with("word/comments") && lower.ends_with(".xml"));
        result.header_count +=
            usize::from(lower.starts_with("word/header") && lower.ends_with(".xml"));
        result.footer_count +=
            usize::from(lower.starts_with("word/footer") && lower.ends_with(".xml"));
        result.has_macros |= lower == "word/vbaproject.bin";
        if is_related_text_part(&lower) {
            result.related_text_parts.push(name);
        }
    }
    for required in ["[Content_Types].xml", "_rels/.rels", MAIN_DOCUMENT_PART] {
        if !names.contains(required) {
            return Err(invalid(
                path,
                format!("required package part {required:?} is missing"),
            ));
        }
    }
    result.related_text_parts.sort();
    Ok(result)
}

fn read_part(
    path: &Path,
    archive: &mut DocumentArchive<'_>,
    part: &str,
) -> Result<Vec<u8>, DocumentError> {
    let mut file = archive
        .by_name(part)
        .map_err(|error| invalid(path, format!("cannot read {part}: {error}")))?;
    if file.size() > MAX_XML_PART_BYTES {
        return Err(DocumentError::PartTooLarge {
            part: part.to_string(),
            actual_bytes: file.size(),
            limit_bytes: MAX_XML_PART_BYTES,
        });
    }
    let mut bytes = Vec::with_capacity(usize::try_from(file.size()).unwrap_or(0));
    file.read_to_end(&mut bytes)
        .map_err(|error| invalid(path, format!("cannot read {part}: {error}")))?;
    Ok(bytes)
}

fn validate_opc_metadata(
    path: &Path,
    archive: &mut DocumentArchive<'_>,
) -> Result<(), DocumentError> {
    let content_types = read_part(path, archive, "[Content_Types].xml")?;
    let mut reader = Reader::from_reader(content_types.as_slice());
    let mut has_main_content_type = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(path, format!("invalid [Content_Types].xml: {error}")))?
        {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"Override" =>
            {
                let (part_name, content_type) = opc_attributes(path, &reader, &element)?;
                has_main_content_type |= part_name.as_deref() == Some("/word/document.xml")
                    && content_type.as_deref()
                        == Some(
                            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml",
                        );
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !has_main_content_type {
        return Err(invalid(
            path,
            "[Content_Types].xml does not declare the DOCX main document part",
        ));
    }

    let relationships = read_part(path, archive, "_rels/.rels")?;
    let mut reader = Reader::from_reader(relationships.as_slice());
    let mut has_office_document_relationship = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid(path, format!("invalid _rels/.rels: {error}")))?
        {
            Event::Start(element) | Event::Empty(element)
                if element.local_name().as_ref() == b"Relationship" =>
            {
                let (target, relationship_type) = opc_attributes(path, &reader, &element)?;
                has_office_document_relationship |= target
                    .as_deref()
                    .is_some_and(|target| target.trim_start_matches('/') == MAIN_DOCUMENT_PART)
                    && relationship_type
                        .as_deref()
                        .is_some_and(|kind| kind.ends_with("/officeDocument"));
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if !has_office_document_relationship {
        return Err(invalid(
            path,
            "_rels/.rels does not target the DOCX main document part",
        ));
    }
    Ok(())
}

fn opc_attributes(
    path: &Path,
    reader: &Reader<&[u8]>,
    element: &quick_xml::events::BytesStart<'_>,
) -> Result<(Option<String>, Option<String>), DocumentError> {
    let mut first = None;
    let mut second = None;
    for attribute in element.attributes() {
        let attribute = attribute.map_err(|error| invalid(path, error.to_string()))?;
        let value = attribute
            .decoded_and_normalized_value(quick_xml::XmlVersion::Implicit1_0, reader.decoder())
            .map_err(|error| invalid(path, error.to_string()))?
            .into_owned();
        match attribute.key.local_name().as_ref() {
            b"PartName" | b"Target" => first = Some(value),
            b"ContentType" | b"Type" => second = Some(value),
            _ => {}
        }
    }
    Ok((first, second))
}

#[derive(Default)]
struct ParsedWordXml {
    text: String,
    paragraph_count: usize,
    table_count: usize,
    tracked_insertions: usize,
    tracked_deletions: usize,
}

fn parse_wordprocessing_xml(
    path: &Path,
    part: &str,
    xml: &[u8],
) -> Result<ParsedWordXml, DocumentError> {
    let mut reader = NsReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut parsed = ParsedWordXml::default();
    let mut in_text = false;
    let mut saw_root = false;
    let mut saw_body = false;
    loop {
        let (namespace, event) = reader
            .read_resolved_event()
            .map_err(|error| invalid(path, format!("invalid XML in {part}: {error}")))?;
        let is_wordprocessingml = matches!(
            namespace,
            ResolveResult::Bound(Namespace(namespace))
                if namespace == WORDPROCESSINGML_NAMESPACE
        );
        match event {
            Event::Start(element) => {
                if !saw_root {
                    saw_root = true;
                    if part == MAIN_DOCUMENT_PART
                        && (!is_wordprocessingml || element.local_name().as_ref() != b"document")
                    {
                        return Err(invalid(
                            path,
                            "word/document.xml root must be a WordprocessingML document element",
                        ));
                    }
                }
                if is_wordprocessingml {
                    match element.local_name().as_ref() {
                        b"body" => saw_body = true,
                        b"t" => in_text = true,
                        b"tbl" => parsed.table_count += 1,
                        b"ins" => parsed.tracked_insertions += 1,
                        b"del" => parsed.tracked_deletions += 1,
                        b"tab" => parsed.text.push('\t'),
                        b"br" | b"cr" => push_separator(&mut parsed.text, '\n'),
                        _ => {}
                    }
                }
            }
            Event::Empty(element) => {
                if !saw_root {
                    saw_root = true;
                    if part == MAIN_DOCUMENT_PART
                        && (!is_wordprocessingml || element.local_name().as_ref() != b"document")
                    {
                        return Err(invalid(
                            path,
                            "word/document.xml root must be a WordprocessingML document element",
                        ));
                    }
                }
                if is_wordprocessingml {
                    match element.local_name().as_ref() {
                        b"body" => saw_body = true,
                        b"tab" => parsed.text.push('\t'),
                        b"br" | b"cr" => push_separator(&mut parsed.text, '\n'),
                        _ => {}
                    }
                }
            }
            Event::Text(text) if in_text => {
                let decoded = text
                    .decode()
                    .map_err(|error| invalid(path, format!("invalid text in {part}: {error}")))?;
                let value = quick_xml::escape::unescape(&decoded)
                    .map_err(|error| invalid(path, format!("invalid entity in {part}: {error}")))?;
                parsed.text.push_str(&value);
            }
            Event::End(element) => {
                if is_wordprocessingml {
                    match element.local_name().as_ref() {
                        b"t" => in_text = false,
                        b"p" => {
                            parsed.paragraph_count += 1;
                            push_separator(&mut parsed.text, '\n');
                        }
                        b"tc" => push_separator(&mut parsed.text, '\t'),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    if part == MAIN_DOCUMENT_PART && (!saw_root || !saw_body) {
        return Err(invalid(
            path,
            "word/document.xml must contain a WordprocessingML body element",
        ));
    }
    while parsed.text.ends_with(['\n', '\t']) {
        parsed.text.pop();
    }
    Ok(parsed)
}

fn safe_part_name(name: &str) -> bool {
    !name.is_empty()
        && !Path::new(name).is_absolute()
        && Path::new(name)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn is_related_text_part(lower: &str) -> bool {
    lower.ends_with(".xml")
        && (lower.starts_with("word/header")
            || lower.starts_with("word/footer")
            || matches!(
                lower,
                "word/comments.xml" | "word/footnotes.xml" | "word/endnotes.xml"
            ))
}

fn push_separator(text: &mut String, separator: char) {
    if !text.is_empty() && !text.ends_with(separator) {
        text.push(separator);
    }
}

fn invalid(path: &Path, message: impl Into<String>) -> DocumentError {
    DocumentError::InvalidDocument {
        path: path.to_path_buf(),
        message: message.into(),
    }
}

fn issue(
    code: impl Into<String>,
    severity: ValidationSeverity,
    message: impl Into<String>,
    location: Option<&str>,
) -> ValidationIssue {
    ValidationIssue {
        code: code.into(),
        severity,
        message: message.into(),
        location: location.map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    use zip::ZipWriter;

    fn sample_docx_with_main(main_document: &str) -> Vec<u8> {
        let cursor = Cursor::new(Vec::new());
        let mut writer = ZipWriter::new(cursor);
        let options = SimpleFileOptions::default();
        let files = [
            (
                "[Content_Types].xml",
                r#"<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/></Types>"#,
            ),
            (
                "_rels/.rels",
                r#"<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/></Relationships>"#,
            ),
            (MAIN_DOCUMENT_PART, main_document),
            (
                "word/header1.xml",
                r#"<w:hdr xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:p><w:r><w:t>Header</w:t></w:r></w:p></w:hdr>"#,
            ),
        ];
        for (name, contents) in files {
            writer.start_file(name, options).expect("start DOCX part");
            writer
                .write_all(contents.as_bytes())
                .expect("write DOCX part");
        }
        writer.finish().expect("finish DOCX").into_inner()
    }

    fn sample_docx() -> Vec<u8> {
        sample_docx_with_main(
            r#"<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t>OpenTopia</w:t></w:r></w:p><w:tbl><w:tr><w:tc><w:p><w:r><w:t>Cell</w:t></w:r></w:p></w:tc></w:tr></w:tbl></w:body></w:document>"#,
        )
    }

    #[test]
    fn inspect_extract_and_validate_docx() {
        let bytes = sample_docx();
        let path = Path::new("sample.docx");
        let inspection = inspect_document(path, &bytes).expect("inspect DOCX");
        assert_eq!(inspection.paragraph_count, 2);
        assert_eq!(inspection.table_count, 1);
        assert_eq!(inspection.header_count, 1);

        let extraction = extract_document_text(path, &bytes, true, 1_000).expect("extract DOCX");
        assert!(extraction.parts[0].text.contains("OpenTopia"));
        assert!(extraction
            .parts
            .iter()
            .any(|part| part.text.contains("Header")));

        let validation = validate_document(path, &bytes);
        assert!(validation.report.valid);
        assert!(validation.inspection.is_some());
    }

    #[test]
    fn validation_reports_broken_packages() {
        let result = validate_document(Path::new("broken.docx"), b"not a zip");
        assert!(!result.report.valid);
        assert!(result.inspection.is_none());
    }

    #[test]
    fn validation_rejects_non_wordprocessingml_main_parts() {
        let bytes = sample_docx_with_main("<garbage/>");
        let result = validate_document(Path::new("broken.docx"), &bytes);
        assert!(!result.report.valid);
        assert!(result.inspection.is_none());
    }
}
