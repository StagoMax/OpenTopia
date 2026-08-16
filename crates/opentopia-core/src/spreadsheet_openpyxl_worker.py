"""Bounded OpenTopia XLSX mutation worker.

The Rust host owns validation, path authorization, staging, size limits, and
result serialization. This worker deliberately has a narrow contract: apply an
already-validated write request with openpyxl and save it to the staged output.
"""

from __future__ import annotations

import json
import sys
from pathlib import Path
from typing import Any

from openpyxl import Workbook, load_workbook


def fail(message: str) -> None:
    print(json.dumps({"ok": False, "error": message}), file=sys.stderr)
    raise SystemExit(1)


def find_sheet(workbook: Workbook, requested: str):
    requested_folded = requested.casefold()
    for worksheet in workbook.worksheets:
        if worksheet.title.casefold() == requested_folded:
            return worksheet
    return None


def decode_value(encoded: dict[str, Any]) -> Any:
    kind = encoded["type"]
    if kind == "blank":
        return None
    if kind in {"string", "integer", "number", "boolean"}:
        return encoded["value"]
    if kind == "formula":
        expression = encoded["value"]["expression"].strip()
        return expression if expression.startswith("=") else f"={expression}"
    fail(f"unsupported cell value type: {kind}")


def create_workbook(sheets: list[dict[str, Any]]) -> Workbook:
    if not sheets:
        fail("a workbook must contain at least one worksheet")
    workbook = Workbook()
    first = workbook.active
    first.title = sheets[0]["name"]
    for sheet in sheets[1:]:
        workbook.create_sheet(sheet["name"])
    return workbook


def main() -> None:
    request = json.load(sys.stdin)
    source = request.get("source")
    output = Path(request["output"])
    sheets = request.get("sheets", [])
    workbook = (
        load_workbook(source, data_only=False, keep_links=True)
        if source
        else create_workbook(sheets)
    )

    for sheet_request in sheets:
        worksheet = find_sheet(workbook, sheet_request["name"])
        if worksheet is None:
            worksheet = workbook.create_sheet(sheet_request["name"])
        visibility = sheet_request.get("visibility")
        if visibility is not None:
            worksheet.sheet_state = {
                "visible": "visible",
                "hidden": "hidden",
                "very_hidden": "veryHidden",
            }[visibility]
        for update in sheet_request.get("cells", []):
            address = update["address"]
            worksheet.cell(
                row=address["row"] + 1,
                column=address["column"] + 1,
            ).value = decode_value(update["value"])

    if not workbook.worksheets:
        fail("a workbook must contain at least one worksheet")
    if not any(sheet.sheet_state == "visible" for sheet in workbook.worksheets):
        fail("a workbook must contain at least one visible worksheet")

    output.parent.mkdir(parents=True, exist_ok=True)
    workbook.save(output)
    print(json.dumps({"ok": True, "output": str(output)}))


if __name__ == "__main__":
    try:
        main()
    except SystemExit:
        raise
    except Exception as error:  # The Rust host converts this to a typed write failure.
        fail(f"{type(error).__name__}: {error}")
