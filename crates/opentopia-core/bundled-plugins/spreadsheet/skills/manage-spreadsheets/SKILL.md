---
name: manage-spreadsheets
description: Inspect, query, edit, transform, validate, and export real spreadsheet files with OpenTopia's native spreadsheet tools. Use for XLS, XLSX, XLSM, XLSB, ODS, CSV, or TSV file tasks; do not use for live control of an external Excel application.
---

# Manage Spreadsheets

Use OpenTopia's `spreadsheet_*` tools for spreadsheet file work. Treat the
currently exposed tool schemas as the source of truth for arguments. Prefer
these native tools over shell scripts or third-party spreadsheet libraries when
they cover the requested operation. All spreadsheet tool argument names use
`snake_case`.

## Decision boundary

- Work only with real paths supplied by attachments, prior tool results, or the
  active workspace. Never invent an attachment path.
- Atomic write tools can edit an existing workbook or create a new destination.
  Set `template` when the output must be rebuilt from an existing template or
  source workbook. The same output path can be rerun without deleting it first.
- Do not promise formatting, charts, merged cells, or other features unless a
  currently exposed tool schema supports them.
- Preserve the user's requested output path and existing workbook structure
  unless the task explicitly authorizes structural changes.

## Workflow

1. Resolve the real input and output paths. When the workbook layout is not
   already known, call `spreadsheet_inspect` before reading or mutating it. For
   template-fill tasks, inspect its bounded `guidance` result before mapping
   fields; it includes data-validation input prompts/rules and cell comments.
2. Read only the smallest relevant regions. Use `spreadsheet_read_ranges` to
   batch A1 ranges across files in one call. Keep the combined request below
   the advertised cell limit and follow its page metadata only when more cells
   are actually needed. Returned rows use compact JSON primitives; special
   values and formulas remain tagged objects. Use
   `spreadsheet_find` or `spreadsheet_filter_rows` instead of loading unrelated
   sheets into model context.
3. Choose the narrowest atomic mutation. Prefer file-to-file operations when
   data can remain inside the spreadsheet layer.
4. After every mutation, verify the affected range or structure with a fresh
   read. Use `spreadsheet_validate` for broad edits, structural changes, or
   explicit acceptance criteria.
5. Return the final real file path and summarize the verified changes. Do not
   include temporary paths or intermediate files unless requested.

## Tool routing

- `spreadsheet_inspect`: discover sheets, used ranges, populated-cell counts,
  data-validation guidance, and cell comments.
- `spreadsheet_read_ranges`: read several bounded A1 ranges across one or more
  workbooks; each workbook is loaded once per call.
- `spreadsheet_find`: locate matching values or formulas.
- `spreadsheet_filter_rows`: count rows matching typed conditions. Its default
  `return_mode: summary` returns only the exact count; request `indices` or
  `rows` only when the model actually needs those values.
- `spreadsheet_write_range`: write a rectangular matrix of typed values or
  formulas. Set `template` to rebuild an output from an existing workbook.
- `spreadsheet_copy_ranges`: copy one or more rectangles between workbooks in
  one atomic transaction without routing cells through the model. Set
  `template` when rebuilding an output from an existing workbook.
- `spreadsheet_copy_rows`: filter source rows and copy mapped columns directly
  into an output in one operation. It does not create a filtered workbook or
  return the row payload to the model. Conditions and mappings use Excel column
  names such as `G` rather than zero-based column indexes. Put optional
  `transforms` on a column mapping so filtering, mapping, and type conversion
  happen in the same operation. Pass `source_header_row`, `source_data_row`, and
  `destination_header_row`; the tool validates referenced headers, infers the
  source data extent, and supports description rows between the source header
  and its first data row. It starts writing below the destination header.
  Value-only copies extend the first destination data row's cell styles down
  generated rows.
- `spreadsheet_fill_ranges`: fill one or more bounded ranges with typed values
  in one atomic transaction; it also accepts `template` for a new destination.
- `spreadsheet_convert_ranges`: apply typed conversions to one or more ranges
  in one atomic transaction; it also accepts `template` for a new destination.
  Date conversions must specify `input_format` and an Excel-invariant
  `output_number_format` such as `yyyy-mm-dd`.
- `spreadsheet_copy_sheet`: copy a worksheet directly between workbooks; use
  `template` when the destination should be rebuilt from another workbook.
- `spreadsheet_delete_rows`: delete rows selected by typed conditions inside
  the spreadsheet layer. Use `match_mode: any` for alternative conditions and
  `template` to create or replace a filtered output without modifying the source.
- `spreadsheet_delete_sheet`: delete one named worksheet.
- `spreadsheet_export_delimited`: export a sheet or range to CSV or TSV.
- `spreadsheet_validate`: reopen the result and check expected sheets, headers,
  row counts, populated cells, value types, and number formats. Validation
  ranges use A1 notation. For sheets with headers, pass
  `header: { row: 1, required: [...] }` using Excel's one-based row number.
  Omit `header` for headerless sheets; never use row `0` as a sentinel.

## Correctness

- Model-facing ranges use ordinary Excel A1 notation such as `A1:K20`, and
  condition/mapping columns use names such as `A` or `BG`.
- Write numbers, booleans, dates, and formulas as typed spreadsheet inputs, not
  display-formatted strings. Keep identifiers such as account numbers or ZIP
  codes as text when leading zeroes matter.
- Treat template validation prompts and comments as input requirements. Apply
  the corresponding typed conversions to the entire destination range, then
  validate both value types and number formats.
- Keep rectangular writes aligned: each row must represent the intended column
  positions. Use several bounded writes when regions are unrelated.
- Prefer `spreadsheet_copy_ranges` and `spreadsheet_copy_sheet` when copying
  existing content. Prefer `spreadsheet_delete_rows` when deletion conditions
  can be evaluated in the file layer.
- Do not silently replace unexpected formula errors with plausible zeroes or
  blanks. Report unsupported or invalid operations precisely.

## Verification and recovery

- For a narrow edit, reread the changed range and compare representative values
  or formulas with the request.
- For a broad or structural edit, inspect the workbook and run
  `spreadsheet_validate` with concrete expectations.
- On the first tool error, read the error, refresh workbook structure if it may
  have changed, correct the smallest argument set, and retry once. Do not loop
  on equivalent failures or rewrite the whole workbook to repair a local error.
