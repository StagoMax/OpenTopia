use super::*;

#[test]
fn read_ranges_returns_multiple_bounded_regions_in_one_result() {
    let directory = TestDirectory::new();
    let workbook = directory.path("ranges.xlsx");
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Data".to_string(),
            visibility: None,
            cells: vec![
                update(0, 0, SpreadsheetCellInput::String("header".to_string())),
                update(10, 2, SpreadsheetCellInput::Integer(42)),
            ],
        }],
    })
    .expect("create range workbook");

    let result = read_ranges(&ReadRangesRequest {
        path: workbook,
        ranges: vec![
            SheetRangeRequest {
                sheet: "Data".to_string(),
                range: range((0, 0), (0, 0)),
            },
            SheetRangeRequest {
                sheet: "Data".to_string(),
                range: range((10, 2), (10, 2)),
            },
        ],
    })
    .expect("read multiple ranges");
    assert_eq!(result.total_cells, 2);
    assert_eq!(result.ranges.len(), 2);
    assert_eq!(
        result.ranges[1].rows[0][0].value,
        SpreadsheetCellValue::Number(42.0)
    );
}

#[test]
fn find_and_filter_return_bounded_structured_rows() {
    let directory = TestDirectory::new();
    let workbook = directory.path("search-filter.xlsx");
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Orders".to_string(),
            visibility: None,
            cells: vec![
                update(0, 0, SpreadsheetCellInput::String("Order".to_string())),
                update(0, 1, SpreadsheetCellInput::String("Status".to_string())),
                update(0, 2, SpreadsheetCellInput::String("Amount".to_string())),
                update(1, 0, SpreadsheetCellInput::String("A-001".to_string())),
                update(1, 1, SpreadsheetCellInput::String("Paid".to_string())),
                update(1, 2, SpreadsheetCellInput::Integer(120)),
                update(2, 0, SpreadsheetCellInput::String("A-002".to_string())),
                update(2, 1, SpreadsheetCellInput::String("Pending".to_string())),
                update(2, 2, SpreadsheetCellInput::Integer(80)),
                update(3, 0, SpreadsheetCellInput::String("B-003".to_string())),
                update(3, 1, SpreadsheetCellInput::String("paid".to_string())),
                update(3, 2, SpreadsheetCellInput::Integer(200)),
                update(
                    4,
                    2,
                    SpreadsheetCellInput::Formula(FormulaInput {
                        expression: "SUM(C2:C4)".to_string(),
                        cached_result: Some("400".to_string()),
                    }),
                ),
            ],
        }],
    })
    .expect("create search/filter workbook");

    let found = find_cells(&FindCellsRequest {
        path: workbook.clone(),
        sheet: Some("Orders".to_string()),
        range: None,
        query: "paid".to_string(),
        match_mode: SpreadsheetTextMatchMode::Exact,
        case_sensitive: false,
        include_formulas: false,
        max_results: 10,
    })
    .expect("find status cells");
    assert_eq!(
        found
            .matches
            .iter()
            .map(|item| item.address)
            .collect::<Vec<_>>(),
        vec![address(1, 1), address(3, 1)]
    );
    assert!(!found.truncated);

    let formula = find_cells(&FindCellsRequest {
        path: workbook.clone(),
        sheet: Some("Orders".to_string()),
        range: None,
        query: "SUM(".to_string(),
        match_mode: SpreadsheetTextMatchMode::Contains,
        case_sensitive: true,
        include_formulas: true,
        max_results: 10,
    })
    .expect("find formula");
    assert_eq!(formula.matches.len(), 1);
    assert!(formula.matches[0].matched_formula);

    let filtered = filter_rows(&FilterRowsRequest {
        path: workbook,
        sheet: "Orders".to_string(),
        range: range((1, 0), (3, 2)),
        conditions: vec![
            SpreadsheetFilterCondition {
                column: 1,
                operator: SpreadsheetFilterOperator::Equals,
                value: Some(SpreadsheetFilterValue::String("paid".to_string())),
                case_sensitive: false,
            },
            SpreadsheetFilterCondition {
                column: 2,
                operator: SpreadsheetFilterOperator::GreaterThanOrEqual,
                value: Some(SpreadsheetFilterValue::Integer(100)),
                case_sensitive: false,
            },
        ],
        match_mode: SpreadsheetFilterMatchMode::All,
        max_results: 10,
    })
    .expect("filter order rows");
    assert_eq!(filtered.matched_row_indices, vec![1, 3]);
    assert_eq!(filtered.rows.len(), 2);
    assert!(!filtered.truncated);
}

#[test]
fn filter_rows_scans_past_read_windows_and_returns_up_to_two_thousand_rows() {
    let directory = TestDirectory::new();
    let workbook = directory.path("large-filter.xlsx");
    let cells = (0..1_501)
        .flat_map(|row| {
            [
                update(row, 0, SpreadsheetCellInput::Integer(row as i64)),
                update(row, 1, SpreadsheetCellInput::String("keep".to_string())),
            ]
        })
        .collect();
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Rows".to_string(),
            visibility: None,
            cells,
        }],
    })
    .expect("create large filter workbook");

    let filtered = filter_rows(&FilterRowsRequest {
        path: workbook,
        sheet: "Rows".to_string(),
        range: range((0, 0), (1_500, 1)),
        conditions: vec![SpreadsheetFilterCondition {
            column: 1,
            operator: SpreadsheetFilterOperator::NotContains,
            value: Some(SpreadsheetFilterValue::String("cancel".to_string())),
            case_sensitive: false,
        }],
        match_mode: SpreadsheetFilterMatchMode::All,
        max_results: 2_000,
    })
    .expect("filter beyond the ordinary read window");
    assert_eq!(filtered.scanned_rows, 1_501);
    assert_eq!(filtered.rows.len(), 1_501);
    assert!(!filtered.truncated);
}

#[test]
fn rejects_range_and_cell_limits() {
    let directory = TestDirectory::new();
    let workbook = directory.path("limits.xlsx");
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells: vec![],
        }],
    })
    .expect("create workbook");

    let error = read_range(&ReadRangeRequest {
        path: workbook,
        sheet: "Sheet1".to_string(),
        range: range((0, 0), (MAX_READ_ROWS as u32, 0)),
    })
    .expect_err("range must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::RangeTooLarge);

    let error = write_workbook(&WriteWorkbookRequest {
        source: None,
        output: directory.path("out-of-bounds.xlsx"),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells: vec![update(
                EXCEL_MAX_ROWS,
                0,
                SpreadsheetCellInput::Boolean(true),
            )],
        }],
    })
    .expect_err("out-of-bounds cell must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::CellOutOfBounds);

    let cells = (0..=MAX_WRITE_UPDATES)
        .map(|row| update(row as u32, 0, SpreadsheetCellInput::Integer(1)))
        .collect();
    let error = write_workbook(&WriteWorkbookRequest {
        source: None,
        output: directory.path("too-many-updates.xlsx"),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells,
        }],
    })
    .expect_err("too many updates must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::TooManyCells);
}

#[test]
fn rejects_oversized_files_but_allows_results_for_the_artifact_boundary() {
    let directory = TestDirectory::new();
    let oversized = directory.path("oversized.xlsx");
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(&oversized)
        .expect("create sparse file");
    file.set_len(MAX_INPUT_FILE_BYTES + 1)
        .expect("extend sparse file");
    drop(file);
    let error = list_sheets(&ListSheetsRequest { path: oversized })
        .expect_err("oversized file must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::FileTooLarge);

    let workbook = directory.path("large-return.xlsx");
    let value = "x".repeat(MAX_CELL_CHARACTERS);
    let cells = (0..40)
        .map(|row| update(row, 0, SpreadsheetCellInput::String(value.clone())))
        .collect();
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells,
        }],
    })
    .expect("create large-return workbook");
    let result = read_range(&ReadRangeRequest {
        path: workbook,
        sheet: "Sheet1".to_string(),
        range: range((0, 0), (39, 0)),
    })
    .expect("large results are handed to the tool-result artifact boundary");
    assert_eq!(result.rows.len(), 40);
}

#[test]
fn reports_unsupported_format_missing_sheet_and_duplicate_update() {
    let directory = TestDirectory::new();
    let unsupported = directory.path("legacy.bin");
    fs::write(&unsupported, b"not a spreadsheet").expect("write unsupported file");
    let error = list_sheets(&ListSheetsRequest { path: unsupported })
        .expect_err("unknown format must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::UnsupportedFormat);

    let xls = directory.path("legacy.xls");
    fs::write(&xls, b"not an xls file").expect("write legacy file");
    let error = list_sheets(&ListSheetsRequest { path: xls })
        .expect_err("malformed legacy workbook must be rejected as invalid");
    assert_eq!(error.code(), SpreadsheetErrorCode::InvalidWorkbook);

    let workbook = directory.path("errors.xlsx");
    write_workbook(&WriteWorkbookRequest {
        source: None,
        output: workbook.clone(),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells: vec![],
        }],
    })
    .expect("create workbook");
    let error = read_range(&ReadRangeRequest {
        path: workbook,
        sheet: "Missing".to_string(),
        range: range((0, 0), (0, 0)),
    })
    .expect_err("missing sheet must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::SheetNotFound);

    let duplicate = update(0, 0, SpreadsheetCellInput::Integer(1));
    let error = write_workbook(&WriteWorkbookRequest {
        source: None,
        output: directory.path("duplicate.xlsx"),
        sheets: vec![SheetWriteRequest {
            name: "Sheet1".to_string(),
            visibility: None,
            cells: vec![duplicate.clone(), duplicate],
        }],
    })
    .expect_err("duplicate update must be rejected");
    assert_eq!(error.code(), SpreadsheetErrorCode::DuplicateCellUpdate);
    assert_eq!(error.info().code, SpreadsheetErrorCode::DuplicateCellUpdate);
}
