use crate::artifact_runtime::{
    ValidationIssue, ValidationReport, ValidationSeverity, MAX_ARTIFACT_INPUT_BYTES,
};
use lopdf::{Dictionary, Document, LoadOptions, Object};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const MAX_PDF_EXTRACT_CHARACTERS: usize = 200_000;
const MAX_PDF_PAGE_CONTENT_BYTES: usize = 16 * 1024 * 1024;
const MAX_PDF_LOAD_STREAM_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfInspection {
    pub path: PathBuf,
    pub file_size_bytes: u64,
    pub version: String,
    pub page_count: usize,
    pub object_count: usize,
    pub encrypted: bool,
    pub has_acro_form: bool,
    pub widget_count: usize,
    pub pages_with_annotations: usize,
    pub embedded_file_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfPageText {
    pub page: u32,
    pub text: String,
    pub character_count: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfExtraction {
    pub path: PathBuf,
    pub pages: Vec<PdfPageText>,
    pub total_characters: usize,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PdfValidation {
    pub path: PathBuf,
    pub report: ValidationReport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub inspection: Option<PdfInspection>,
}

#[derive(Debug, thiserror::Error)]
pub enum PdfError {
    #[error("unsupported PDF path {0}: expected a .pdf file")]
    UnsupportedFormat(PathBuf),
    #[error("PDF input is {actual_bytes} bytes; limit is {limit_bytes} bytes")]
    FileTooLarge { actual_bytes: u64, limit_bytes: u64 },
    #[error("invalid PDF {path}: {message}")]
    InvalidPdf { path: PathBuf, message: String },
    #[error("PDF page {page} is outside the page range 1..={page_count}")]
    PageOutOfRange { page: u32, page_count: usize },
    #[error("maxCharacters must be between 1 and {MAX_PDF_EXTRACT_CHARACTERS}")]
    InvalidCharacterLimit,
}

pub fn inspect_pdf(path: &Path, bytes: &[u8]) -> Result<PdfInspection, PdfError> {
    validate_pdf_input(path, bytes)?;
    let document = load_document(path, bytes)?;
    Ok(inspect_document(path, bytes.len() as u64, &document))
}

pub fn extract_pdf_text(
    path: &Path,
    bytes: &[u8],
    requested_pages: &[u32],
    max_characters: usize,
) -> Result<PdfExtraction, PdfError> {
    validate_pdf_input(path, bytes)?;
    if !(1..=MAX_PDF_EXTRACT_CHARACTERS).contains(&max_characters) {
        return Err(PdfError::InvalidCharacterLimit);
    }
    let document = load_document(path, bytes)?;
    let page_count = document.get_pages().len();
    let pages = if requested_pages.is_empty() {
        (1..=u32::try_from(page_count).unwrap_or(u32::MAX)).collect::<Vec<_>>()
    } else {
        requested_pages.to_vec()
    };
    let mut output = Vec::with_capacity(pages.len());
    let mut remaining = max_characters;
    let mut total_characters = 0_usize;
    let mut truncated = false;
    for page in pages {
        if page == 0 || usize::try_from(page).unwrap_or(usize::MAX) > page_count {
            return Err(PdfError::PageOutOfRange { page, page_count });
        }
        if remaining == 0 {
            truncated = true;
            break;
        }
        let text = document
            .extract_text_with_limit(&[page], MAX_PDF_PAGE_CONTENT_BYTES)
            .map_err(|error| PdfError::InvalidPdf {
                path: path.to_path_buf(),
                message: format!("failed to extract page {page}: {error}"),
            })?;
        let character_count = text.chars().count();
        let page_truncated = character_count > remaining;
        let text = if page_truncated {
            text.chars().take(remaining).collect()
        } else {
            text
        };
        let returned = text.chars().count();
        total_characters = total_characters.saturating_add(returned);
        remaining = remaining.saturating_sub(returned);
        truncated |= page_truncated;
        output.push(PdfPageText {
            page,
            text,
            character_count,
            truncated: page_truncated,
        });
        if page_truncated {
            break;
        }
    }
    Ok(PdfExtraction {
        path: path.to_path_buf(),
        pages: output,
        total_characters,
        truncated,
    })
}

pub fn validate_pdf(path: &Path, bytes: &[u8]) -> PdfValidation {
    let mut issues = Vec::new();
    if let Err(error) = validate_pdf_input(path, bytes) {
        issues.push(issue(
            "invalid_input",
            ValidationSeverity::Error,
            error.to_string(),
        ));
        return PdfValidation {
            path: path.to_path_buf(),
            report: ValidationReport::from_issues(issues),
            inspection: None,
        };
    }
    let document = match load_document(path, bytes) {
        Ok(document) => document,
        Err(error) => {
            issues.push(issue(
                "invalid_pdf",
                ValidationSeverity::Error,
                error.to_string(),
            ));
            return PdfValidation {
                path: path.to_path_buf(),
                report: ValidationReport::from_issues(issues),
                inspection: None,
            };
        }
    };
    let inspection = inspect_document(path, bytes.len() as u64, &document);
    if document.catalog().is_err() {
        issues.push(issue(
            "missing_catalog",
            ValidationSeverity::Error,
            "the PDF catalog cannot be resolved",
        ));
    }
    if inspection.page_count == 0 {
        issues.push(issue(
            "no_pages",
            ValidationSeverity::Error,
            "the PDF does not contain any pages",
        ));
    }
    if inspection.encrypted {
        issues.push(issue(
            "encrypted",
            ValidationSeverity::Warning,
            "the PDF is encrypted; extraction and rendering may require a password",
        ));
    }
    if inspection.has_acro_form {
        issues.push(issue(
            "acro_form_present",
            ValidationSeverity::Info,
            "the PDF contains an AcroForm; validate field values and appearances after edits",
        ));
        let field_count = acro_form_field_count(&document).unwrap_or(0);
        if field_count == 0 {
            issues.push(issue(
                "acro_form_fields_missing",
                ValidationSeverity::Warning,
                "the AcroForm does not expose a non-empty Fields array",
            ));
        }
        if inspection.widget_count == 0 {
            issues.push(issue(
                "acro_form_widgets_missing",
                ValidationSeverity::Warning,
                "the AcroForm has no Widget annotations",
            ));
        }
        let missing_appearances = widget_missing_appearance_count(&document);
        if missing_appearances > 0 {
            issues.push(issue(
                "widget_appearances_missing",
                ValidationSeverity::Warning,
                format!(
                    "{missing_appearances} Widget annotations do not define an appearance dictionary"
                ),
            ));
        }
    }
    PdfValidation {
        path: path.to_path_buf(),
        report: ValidationReport::from_issues(issues),
        inspection: Some(inspection),
    }
}

fn validate_pdf_input(path: &Path, bytes: &[u8]) -> Result<(), PdfError> {
    let is_pdf = path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case("pdf"));
    if !is_pdf {
        return Err(PdfError::UnsupportedFormat(path.to_path_buf()));
    }
    if bytes.len() as u64 > MAX_ARTIFACT_INPUT_BYTES {
        return Err(PdfError::FileTooLarge {
            actual_bytes: bytes.len() as u64,
            limit_bytes: MAX_ARTIFACT_INPUT_BYTES,
        });
    }
    Ok(())
}

fn load_document(path: &Path, bytes: &[u8]) -> Result<Document, PdfError> {
    Document::load_mem_with_options(
        bytes,
        LoadOptions::with_max_decompressed_size(MAX_PDF_LOAD_STREAM_BYTES),
    )
    .map_err(|error| PdfError::InvalidPdf {
        path: path.to_path_buf(),
        message: error.to_string(),
    })
}

fn inspect_document(path: &Path, file_size_bytes: u64, document: &Document) -> PdfInspection {
    let pages = document.get_pages();
    let pages_with_annotations = pages
        .values()
        .filter(|page_id| {
            document
                .get_dictionary(**page_id)
                .is_ok_and(|page| page.get(b"Annots").is_ok())
        })
        .count();
    let mut widget_count = 0_usize;
    let mut embedded_file_count = 0_usize;
    for object in document.objects.values() {
        let Some(dictionary) = object_dictionary(object) else {
            continue;
        };
        if dictionary_name(dictionary, b"Subtype") == Some(b"Widget".as_slice()) {
            widget_count += 1;
        }
        if matches!(
            dictionary_name(dictionary, b"Type"),
            Some(name) if name == b"Filespec" || name == b"EmbeddedFile"
        ) {
            embedded_file_count += 1;
        }
    }
    PdfInspection {
        path: path.to_path_buf(),
        file_size_bytes,
        version: document.version.clone(),
        page_count: pages.len(),
        object_count: document.objects.len(),
        encrypted: document.is_encrypted(),
        has_acro_form: document
            .catalog()
            .is_ok_and(|catalog| catalog.get(b"AcroForm").is_ok()),
        widget_count,
        pages_with_annotations,
        embedded_file_count,
    }
}

fn object_dictionary(object: &Object) -> Option<&Dictionary> {
    match object {
        Object::Dictionary(dictionary) => Some(dictionary),
        Object::Stream(stream) => Some(&stream.dict),
        _ => None,
    }
}

fn dictionary_name<'a>(dictionary: &'a Dictionary, key: &[u8]) -> Option<&'a [u8]> {
    dictionary.get(key).ok()?.as_name().ok()
}

fn acro_form_field_count(document: &Document) -> Option<usize> {
    let catalog = document.catalog().ok()?;
    let acro_form = resolved_object(document, catalog.get(b"AcroForm").ok()?)?;
    let fields = resolved_object(document, acro_form.as_dict().ok()?.get(b"Fields").ok()?)?;
    fields.as_array().ok().map(Vec::len)
}

fn widget_missing_appearance_count(document: &Document) -> usize {
    document
        .objects
        .values()
        .filter_map(object_dictionary)
        .filter(|dictionary| dictionary_name(dictionary, b"Subtype") == Some(b"Widget".as_slice()))
        .filter(|dictionary| dictionary.get(b"AP").is_err())
        .count()
}

fn resolved_object<'a>(document: &'a Document, object: &'a Object) -> Option<&'a Object> {
    document.dereference(object).ok().map(|(_, object)| object)
}

fn issue(
    code: impl Into<String>,
    severity: ValidationSeverity,
    message: impl Into<String>,
) -> ValidationIssue {
    ValidationIssue {
        code: code.into(),
        severity,
        message: message.into(),
        location: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lopdf::{dictionary, Object, Stream};

    fn sample_pdf(text: &str) -> Vec<u8> {
        let mut document = Document::with_version("1.7");
        let pages_id = document.new_object_id();
        let page_id = document.new_object_id();
        let font_id = document.add_object(dictionary! {
            "Type" => "Font",
            "Subtype" => "Type1",
            "BaseFont" => "Helvetica",
        });
        let resources_id = document.add_object(dictionary! {
            "Font" => dictionary! { "F1" => font_id },
        });
        let content = lopdf::content::Content {
            operations: vec![
                lopdf::content::Operation::new("BT", vec![]),
                lopdf::content::Operation::new("Tf", vec!["F1".into(), 12.into()]),
                lopdf::content::Operation::new("Td", vec![48.into(), 760.into()]),
                lopdf::content::Operation::new("Tj", vec![Object::string_literal(text)]),
                lopdf::content::Operation::new("ET", vec![]),
            ],
        };
        let content_id = document.add_object(Stream::new(
            dictionary! {},
            content.encode().expect("encode PDF content"),
        ));
        document.objects.insert(
            page_id,
            Object::Dictionary(dictionary! {
                "Type" => "Page",
                "Parent" => pages_id,
                "Contents" => content_id,
                "Resources" => resources_id,
                "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            }),
        );
        document.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {
                "Type" => "Pages",
                "Kids" => vec![page_id.into()],
                "Count" => 1,
            }),
        );
        let catalog_id = document.add_object(dictionary! {
            "Type" => "Catalog",
            "Pages" => pages_id,
        });
        document.trailer.set("Root", catalog_id);
        let mut bytes = Vec::new();
        document.save_to(&mut bytes).expect("save sample PDF");
        bytes
    }

    #[test]
    fn inspect_extract_and_validate_pdf() {
        let bytes = sample_pdf("OpenTopia PDF");
        let path = Path::new("sample.pdf");
        let inspection = inspect_pdf(path, &bytes).expect("inspect PDF");
        assert_eq!(inspection.page_count, 1);
        assert!(!inspection.encrypted);

        let extraction = extract_pdf_text(path, &bytes, &[1], 1_000).expect("extract PDF");
        assert!(extraction.pages[0].text.contains("OpenTopia PDF"));
        assert!(!extraction.truncated);

        let validation = validate_pdf(path, &bytes);
        assert!(validation.report.valid);
        assert!(validation.inspection.is_some());
    }

    #[test]
    fn validation_reports_invalid_bytes_without_failing_the_call() {
        let result = validate_pdf(Path::new("broken.pdf"), b"not a pdf");
        assert!(!result.report.valid);
        assert!(result.inspection.is_none());
    }
}
