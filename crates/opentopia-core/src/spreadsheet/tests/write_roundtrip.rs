use super::*;

#[test]
fn create_inspect_read_and_update_roundtrip() {
    let directory = TestDirectory::new();
    let original = directory.path("original.xlsx");
    let updated = directory.path("updated.xlsx");

    let created = write_workbook(&WriteWorkbookRequest {
        source: None,
        output: original.clone(),
        sheets: vec![
            SheetWriteRequest {
                name: "Data".to_string(),
                visibility: None,
                cells: vec![
                    update(0, 0, SpreadsheetCellInput::String("label".to_string())),
                    update(1, 0, SpreadsheetCellInput::Integer(42)),
                    update(
                        1,
                        1,
                        SpreadsheetCellInput::Formula(FormulaInput {
                            expression: "A2*2".to_string(),
                            cached_result: Some("84".to_string()),
                        }),
                    ),
                ],
            },
            SheetWriteRequest {
                name: "Archive".to_string(),
                visibility: Some(SheetVisibility::Hidden),
                cells: vec![],
            },
        ],
    })
    .expect("create workbook");
    assert_eq!(created.sheet_count, 2);
    assert_eq!(created.output_cells, 3);

    let listed = list_sheets(&ListSheetsRequest {
        path: original.clone(),
    })
    .expect("list sheets");
    assert_eq!(listed.sheets.len(), 2);
    assert_eq!(listed.sheets[1].visibility, SheetVisibility::Hidden);

    let inspected = inspect_workbook(&InspectWorkbookRequest {
        path: original.clone(),
    })
    .expect("inspect workbook");
    assert_eq!(inspected.populated_cells, 3);
    assert_eq!(inspected.sheets[0].used_range, Some(range((0, 0), (1, 1))));

    let read = read_range(&ReadRangeRequest {
        path: original.clone(),
        sheet: "Data".to_string(),
        range: range((0, 0), (1, 1)),
    })
    .expect("read range");
    assert_eq!(
        read.rows[0][0].value,
        SpreadsheetCellValue::String("label".to_string())
    );
    assert_eq!(read.rows[1][0].value, SpreadsheetCellValue::Number(42.0));
    assert!(read.rows[1][1]
        .formula
        .as_deref()
        .is_some_and(|formula| formula.contains("A2*2")));

    write_workbook(&WriteWorkbookRequest {
        source: Some(original),
        output: updated.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Data".to_string(),
            visibility: None,
            cells: vec![
                update(1, 0, SpreadsheetCellInput::Integer(43)),
                update(0, 2, SpreadsheetCellInput::Boolean(true)),
            ],
        }],
    })
    .expect("update workbook");

    let read = read_range(&ReadRangeRequest {
        path: updated,
        sheet: "Data".to_string(),
        range: range((0, 0), (1, 2)),
    })
    .expect("read updated range");
    assert_eq!(read.rows[1][0].value, SpreadsheetCellValue::Number(43.0));
    assert_eq!(read.rows[0][2].value, SpreadsheetCellValue::Boolean(true));
    assert!(read.rows[1][1].formula.is_some());
}

#[test]
fn legacy_xls_reports_missing_writer_without_creating_a_file() {
    let directory = TestDirectory::new();
    let output = directory.path("legacy.xls");
    let error = write_workbook_preferred(&WriteWorkbookRequest {
        source: None,
        output: output.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Data".to_string(),
            visibility: None,
            cells: vec![update(
                0,
                0,
                SpreadsheetCellInput::String("legacy".to_string()),
            )],
        }],
    })
    .expect_err("XLS writer is unavailable");
    assert_eq!(error.code(), SpreadsheetErrorCode::UnsupportedFormat);
    assert!(!output.exists());
}

#[test]
fn delimited_write_uses_the_model_chosen_output_extension() {
    let directory = TestDirectory::new();
    let csv = directory.path("data.csv");
    let tsv = directory.path("data.tsv");

    let created = write_workbook_preferred(&WriteWorkbookRequest {
        source: None,
        output: csv.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Data".to_string(),
            visibility: None,
            cells: vec![
                update(0, 0, SpreadsheetCellInput::String("name".to_string())),
                update(1, 0, SpreadsheetCellInput::String("alpha".to_string())),
                update(1, 1, SpreadsheetCellInput::Integer(7)),
            ],
        }],
    })
    .expect("create CSV");
    assert_eq!(created.backend, SpreadsheetWriteBackend::Delimited);
    assert_eq!(
        fs::read_to_string(&csv).expect("read CSV"),
        "name,\nalpha,7\n"
    );

    write_workbook_preferred(&WriteWorkbookRequest {
        source: Some(csv),
        output: tsv.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Data".to_string(),
            visibility: None,
            cells: vec![update(
                0,
                1,
                SpreadsheetCellInput::String("count".to_string()),
            )],
        }],
    })
    .expect("convert CSV input to TSV output");
    assert_eq!(
        fs::read_to_string(tsv).expect("read TSV"),
        "name\tcount\nalpha\t7\n"
    );
}

#[test]
fn template_patch_preserves_styles_and_non_worksheet_parts() {
    let directory = TestDirectory::new();
    let template = directory.path("styled-template.xlsx");
    let output = directory.path("styled-output.xlsx");
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Orders").expect("set sheet name");
    let format = Format::new().set_bold().set_background_color(Color::Yellow);
    worksheet
        .write_with_format(0, 0, "old", &format)
        .expect("write formatted template cell");
    worksheet
        .set_column_width(0, 28)
        .expect("set template width");
    workbook.save(&template).expect("save template");

    let styles_before = zip_part(&template, "xl/styles.xml");
    let workbook_before = zip_part(&template, "xl/workbook.xml");
    let result = write_workbook(&WriteWorkbookRequest {
        source: Some(template.clone()),
        output: output.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Orders".to_string(),
            visibility: None,
            cells: vec![
                update(0, 0, SpreadsheetCellInput::String("new".to_string())),
                CellUpdate {
                    address: address(10, 2),
                    value: SpreadsheetCellInput::Integer(17),
                    style_from: Some(address(0, 0)),
                },
            ],
        }],
    })
    .expect("patch template");

    assert!(result.preserved_template_parts);
    assert!(!result.rebuilt_from_source);
    assert_eq!(zip_part(&output, "xl/styles.xml"), styles_before);
    assert_eq!(zip_part(&output, "xl/workbook.xml"), workbook_before);
    let worksheet_xml = String::from_utf8(zip_part(&output, "xl/worksheets/sheet1.xml"))
        .expect("worksheet XML is UTF-8");
    assert!(worksheet_xml.contains("<dimension ref=\"A1:C11\""));
    assert!(worksheet_xml.contains("s=\"1\""));
    assert!(worksheet_xml.contains("<c r=\"C11\" s=\"1\""));
    assert!(worksheet_xml.contains(">new<"));
    let read = read_range(&ReadRangeRequest {
        path: output,
        sheet: "Orders".to_string(),
        range: range((0, 0), (0, 0)),
    })
    .expect("read patched template");
    assert_eq!(
        read.rows[0][0].value,
        SpreadsheetCellValue::String("new".to_string())
    );
    let appended = read_range(&ReadRangeRequest {
        path: read.path,
        sheet: "Orders".to_string(),
        range: range((10, 2), (10, 2)),
    })
    .expect("read cell appended beyond the template dimension");
    assert_eq!(
        appended.rows[0][0].value,
        SpreadsheetCellValue::Number(17.0)
    );
}

#[test]
fn openpyxl_backend_round_trips_structural_changes_when_available() {
    let Ok(python) = crate::office_runtime::OfficeRuntime::shared().python_for_openpyxl() else {
        return;
    };
    let directory = TestDirectory::new();
    let template = directory.path("openpyxl-template.xlsx");
    let output = directory.path("openpyxl-output.xlsx");
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet.set_name("Orders").expect("set sheet name");
    let format = Format::new().set_bold().set_background_color(Color::Yellow);
    worksheet
        .write_with_format(0, 0, "old", &format)
        .expect("write formatted template cell");
    workbook.save(&template).expect("save template");

    let result = write_workbook_openpyxl(
        &WriteWorkbookRequest {
            source: Some(template),
            output: output.clone(),
            sheets: vec![
                SheetWriteRequest {
                    name: "Orders".to_string(),
                    visibility: None,
                    cells: vec![update(
                        0,
                        0,
                        SpreadsheetCellInput::String("updated".to_string()),
                    )],
                },
                SheetWriteRequest {
                    name: "Review".to_string(),
                    visibility: None,
                    cells: vec![update(
                        0,
                        0,
                        SpreadsheetCellInput::String("ready".to_string()),
                    )],
                },
            ],
        },
        &python.executable,
    )
    .expect("run openpyxl backend");
    assert_eq!(result.backend, SpreadsheetWriteBackend::Openpyxl);
    assert_eq!(result.sheet_count, 2);
    assert!(result.rebuilt_from_source);
    assert!(!result.preserved_template_parts);
    let listed = list_sheets(&ListSheetsRequest {
        path: output.clone(),
    })
    .expect("list openpyxl output sheets");
    assert_eq!(listed.sheets.len(), 2);
    let read = read_range(&ReadRangeRequest {
        path: output.clone(),
        sheet: "Review".to_string(),
        range: range((0, 0), (0, 0)),
    })
    .expect("read added sheet");
    assert_eq!(
        read.rows[0][0].value,
        SpreadsheetCellValue::String("ready".to_string())
    );
    let worksheet_xml = String::from_utf8(zip_part(&output, "xl/worksheets/sheet1.xml"))
        .expect("worksheet XML is UTF-8");
    assert!(worksheet_xml.contains("s=\"1\""));
}
