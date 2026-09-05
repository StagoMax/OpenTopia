use super::ooxml::{
    invalid_ooxml_xml, normalize_workbook_part_target, open_ooxml_package, read_zip_part,
    workbook_relationship_targets, workbook_sheet_relationships, xml_attribute, OoxmlArchive,
};
use super::{
    Path, SpreadsheetCellComment, SpreadsheetDataValidation, SpreadsheetError,
    SpreadsheetFileFormat, WorkbookGuidance, XmlEvent, XmlReader,
};
use quick_xml::events::BytesStart;
use quick_xml::XmlVersion;
use std::io::Read;

const MAX_GUIDANCE_ITEMS: usize = 128;

pub(super) fn inspect_workbook_guidance(path: &Path) -> Result<WorkbookGuidance, SpreadsheetError> {
    if !SpreadsheetFileFormat::from_path(path).is_some_and(SpreadsheetFileFormat::is_ooxml) {
        return Ok(WorkbookGuidance::default());
    }

    let mut archive = open_ooxml_package(path)?;
    let workbook_xml = read_zip_part(&mut archive, "xl/workbook.xml", path)?;
    let relationships_xml = read_zip_part(&mut archive, "xl/_rels/workbook.xml.rels", path)?;
    let sheets = workbook_sheet_relationships(&workbook_xml, path)?;
    let targets = workbook_relationship_targets(&relationships_xml, path)?;
    let mut result = WorkbookGuidance::default();

    for (sheet, relationship_id) in sheets {
        let Some(target) = targets.get(&relationship_id) else {
            continue;
        };
        let worksheet_part = normalize_workbook_part_target(target);
        let worksheet_xml = read_zip_part(&mut archive, &worksheet_part, path)?;
        append_data_validations(&mut result, &sheet, &worksheet_xml, path)?;
        append_comments(&mut result, &sheet, &worksheet_part, path, &mut archive)?;
    }
    Ok(result)
}

fn append_data_validations(
    result: &mut WorkbookGuidance,
    sheet: &str,
    xml: &[u8],
    source: &Path,
) -> Result<(), SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut current = None::<SpreadsheetDataValidation>;
    let mut text_target = None::<&'static str>;
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "worksheet data validations", error))?
        {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"dataValidation" => {
                if result.data_validations.len() >= MAX_GUIDANCE_ITEMS {
                    result.truncated = true;
                } else {
                    current = Some(data_validation(&reader, &element, sheet)?);
                }
            }
            XmlEvent::Empty(element) if element.local_name().as_ref() == b"dataValidation" => {
                if result.data_validations.len() >= MAX_GUIDANCE_ITEMS {
                    result.truncated = true;
                } else {
                    result
                        .data_validations
                        .push(data_validation(&reader, &element, sheet)?);
                }
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"formula1" => {
                text_target = Some("formula1")
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"formula2" => {
                text_target = Some("formula2")
            }
            XmlEvent::Text(text) if current.is_some() && text_target.is_some() => {
                let value = text
                    .xml_content(XmlVersion::Implicit1_0)
                    .map_err(|error| {
                        invalid_ooxml_xml(source, "worksheet data validations", error)
                    })?
                    .into_owned();
                if let Some(item) = current.as_mut() {
                    match text_target {
                        Some("formula1") => append_text(&mut item.formula1, &value),
                        Some("formula2") => append_text(&mut item.formula2, &value),
                        _ => {}
                    }
                }
            }
            XmlEvent::End(element)
                if matches!(element.local_name().as_ref(), b"formula1" | b"formula2") =>
            {
                text_target = None
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"dataValidation" => {
                if let Some(item) = current.take() {
                    result.data_validations.push(item);
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok(())
}

fn data_validation(
    reader: &XmlReader<&[u8]>,
    element: &BytesStart<'_>,
    sheet: &str,
) -> Result<SpreadsheetDataValidation, SpreadsheetError> {
    let sqref = xml_attribute(reader, element, b"sqref")?.unwrap_or_default();
    Ok(SpreadsheetDataValidation {
        sheet: sheet.to_string(),
        ranges: sqref.split_whitespace().map(str::to_string).collect(),
        validation_type: xml_attribute(reader, element, b"type")?,
        operator: xml_attribute(reader, element, b"operator")?,
        allow_blank: xml_attribute(reader, element, b"allowBlank")?
            .and_then(|value| parse_ooxml_bool(&value)),
        prompt_title: xml_attribute(reader, element, b"promptTitle")?,
        prompt: xml_attribute(reader, element, b"prompt")?,
        error_title: xml_attribute(reader, element, b"errorTitle")?,
        error: xml_attribute(reader, element, b"error")?,
        formula1: None,
        formula2: None,
    })
}

fn append_comments(
    result: &mut WorkbookGuidance,
    sheet: &str,
    worksheet_part: &str,
    source: &Path,
    archive: &mut OoxmlArchive,
) -> Result<(), SpreadsheetError> {
    let relationships_part = relationship_part_name(worksheet_part);
    let relationships_xml = {
        let Ok(mut part) = archive.by_name(&relationships_part) else {
            return Ok(());
        };
        let mut bytes = Vec::new();
        part.read_to_end(&mut bytes)
            .map_err(|source_error| SpreadsheetError::Io {
                operation: "read",
                path: source.to_path_buf(),
                source: source_error,
            })?;
        bytes
    };
    let Some(comments_target) = comments_relationship_target(&relationships_xml, source)? else {
        return Ok(());
    };
    let comments_part = resolve_related_part(worksheet_part, &comments_target);
    let comments_xml = read_zip_part(archive, &comments_part, source)?;
    let remaining = MAX_GUIDANCE_ITEMS.saturating_sub(result.comments.len());
    let (mut comments, truncated) = parse_comments(&comments_xml, sheet, remaining, source)?;
    result.comments.append(&mut comments);
    result.truncated |= truncated;
    Ok(())
}

fn comments_relationship_target(
    xml: &[u8],
    source: &Path,
) -> Result<Option<String>, SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "worksheet relationships", error))?
        {
            XmlEvent::Start(element) | XmlEvent::Empty(element)
                if element.local_name().as_ref() == b"Relationship" =>
            {
                let relation_type = xml_attribute(&reader, &element, b"Type")?;
                if relation_type
                    .as_deref()
                    .is_some_and(|value| value.ends_with("/comments"))
                {
                    return xml_attribute(&reader, &element, b"Target");
                }
            }
            XmlEvent::Eof => return Ok(None),
            _ => {}
        }
    }
}

fn parse_comments(
    xml: &[u8],
    sheet: &str,
    limit: usize,
    source: &Path,
) -> Result<(Vec<SpreadsheetCellComment>, bool), SpreadsheetError> {
    let mut reader = XmlReader::from_reader(xml);
    reader.config_mut().trim_text(false);
    let mut authors = Vec::<String>::new();
    let mut in_author = false;
    let mut in_comment_text = false;
    let mut author = String::new();
    let mut current = None::<(String, Option<usize>, String)>;
    let mut comments = Vec::new();
    let mut truncated = false;
    loop {
        match reader
            .read_event()
            .map_err(|error| invalid_ooxml_xml(source, "comments", error))?
        {
            XmlEvent::Start(element) if element.local_name().as_ref() == b"author" => {
                in_author = true;
                author.clear();
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"comment" => {
                let cell = xml_attribute(&reader, &element, b"ref")?.unwrap_or_default();
                let author_id = xml_attribute(&reader, &element, b"authorId")?
                    .and_then(|value| value.parse::<usize>().ok());
                current = Some((cell, author_id, String::new()));
            }
            XmlEvent::Start(element) if element.local_name().as_ref() == b"text" => {
                in_comment_text = current.is_some();
            }
            XmlEvent::Text(text) if in_author || in_comment_text => {
                let value = text
                    .xml_content(XmlVersion::Implicit1_0)
                    .map_err(|error| invalid_ooxml_xml(source, "comments", error))?;
                if in_author {
                    author.push_str(&value);
                } else if let Some((_, _, body)) = current.as_mut() {
                    body.push_str(&value);
                }
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"author" => {
                authors.push(std::mem::take(&mut author));
                in_author = false;
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"text" => {
                in_comment_text = false;
            }
            XmlEvent::End(element) if element.local_name().as_ref() == b"comment" => {
                if let Some((cell, author_id, text)) = current.take() {
                    if comments.len() < limit {
                        let author = author_id.and_then(|id| authors.get(id).cloned());
                        let text = normalize_comment_text(text, author.as_deref());
                        comments.push(SpreadsheetCellComment {
                            sheet: sheet.to_string(),
                            cell,
                            author,
                            text,
                        });
                    } else {
                        truncated = true;
                    }
                }
            }
            XmlEvent::Eof => break,
            _ => {}
        }
    }
    Ok((comments, truncated))
}

fn relationship_part_name(owner_part: &str) -> String {
    let (directory, file) = owner_part.rsplit_once('/').unwrap_or(("", owner_part));
    if directory.is_empty() {
        format!("_rels/{file}.rels")
    } else {
        format!("{directory}/_rels/{file}.rels")
    }
}

fn resolve_related_part(owner_part: &str, target: &str) -> String {
    if target.starts_with('/') {
        return target.trim_start_matches('/').to_string();
    }
    let directory = owner_part
        .rsplit_once('/')
        .map_or("", |(directory, _)| directory);
    let mut parts = directory
        .split('/')
        .filter(|part| !part.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    for part in target.replace('\\', "/").split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part.to_string()),
        }
    }
    parts.join("/")
}

fn parse_ooxml_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" => Some(true),
        "0" | "false" => Some(false),
        _ => None,
    }
}

fn append_text(target: &mut Option<String>, value: &str) {
    target.get_or_insert_with(String::new).push_str(value);
}

fn normalize_comment_text(text: String, author: Option<&str>) -> String {
    let Some(author) = author.filter(|author| !author.is_empty()) else {
        return text;
    };
    text.strip_prefix(&format!("{author}:\n"))
        .or_else(|| text.strip_prefix(&format!("{author}:\r\n")))
        .unwrap_or(&text)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::{DataValidation, DataValidationRule, Note, Workbook};
    use uuid::Uuid;

    #[test]
    fn resolves_worksheet_relationship_targets() {
        assert_eq!(
            resolve_related_part("xl/worksheets/sheet1.xml", "../comments1.xml"),
            "xl/comments1.xml"
        );
    }

    #[test]
    fn reads_validation_prompts_and_cell_notes() {
        let directory = std::env::temp_dir().join(format!("opentopia-guidance-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join("template.xlsx");
        let mut workbook = Workbook::new();
        let worksheet = workbook.add_worksheet();
        worksheet.set_name("Template").unwrap();
        worksheet.write_string(0, 1, "Date").unwrap();
        let validation = DataValidation::new()
            .allow_whole_number(DataValidationRule::Between(1, 100))
            .set_input_title("Required format")
            .unwrap()
            .set_input_message("Enter a whole number")
            .unwrap();
        worksheet
            .add_data_validation(1, 1, 10, 1, &validation)
            .unwrap();
        let note = Note::new("Template note").set_author("OpenTopia");
        worksheet.insert_note(2, 0, &note).unwrap();
        workbook.save(&path).unwrap();

        let guidance = inspect_workbook_guidance(&path).unwrap();
        assert_eq!(guidance.data_validations.len(), 1);
        assert_eq!(guidance.data_validations[0].sheet, "Template");
        assert_eq!(guidance.data_validations[0].ranges, ["B2:B11"]);
        assert_eq!(
            guidance.data_validations[0].prompt.as_deref(),
            Some("Enter a whole number")
        );
        assert_eq!(guidance.comments.len(), 1);
        assert_eq!(guidance.comments[0].cell, "A3");
        assert_eq!(guidance.comments[0].author.as_deref(), Some("OpenTopia"));
        assert_eq!(guidance.comments[0].text, "Template note");
        assert!(!guidance.truncated);

        std::fs::remove_dir_all(directory).unwrap();
    }
}
