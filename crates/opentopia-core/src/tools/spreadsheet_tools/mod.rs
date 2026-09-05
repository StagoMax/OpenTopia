//! Model-facing spreadsheet tools.
//!
//! Each tool has one fixed contract and operates on real filesystem paths.
//! Progressive disclosure is owned by the global tool catalog; this module
//! deliberately has no document handles, operation envelopes, or selections.

mod common;
mod read;
mod structure;
mod write;

pub use read::{
    SpreadsheetFilterRowsTool, SpreadsheetFindTool, SpreadsheetInspectTool,
    SpreadsheetReadRangesTool, SpreadsheetValidateTool,
};
pub use structure::{
    SpreadsheetCopySheetTool, SpreadsheetDeleteRowsTool, SpreadsheetDeleteSheetTool,
};
pub use write::{
    SpreadsheetConvertRangesTool, SpreadsheetCopyRangesTool, SpreadsheetCopyRowsTool,
    SpreadsheetExportDelimitedTool, SpreadsheetFillRangesTool, SpreadsheetWriteRangeTool,
};
