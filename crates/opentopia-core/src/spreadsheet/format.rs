use std::path::Path;

/// Spreadsheet formats recognized by the offline attachment, preview, and
/// native spreadsheet layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpreadsheetFileFormat {
    Xls,
    Xlsx,
    Xlsm,
    Xlsb,
    Xltx,
    Xltm,
    Ods,
    Csv,
    Tsv,
}

impl SpreadsheetFileFormat {
    pub const ATTACHMENT_EXTENSIONS: &'static [&'static str] = &[
        "xls", "xlsx", "xlsm", "xlsb", "xltx", "xltm", "ods", "csv", "tsv", "tab",
    ];

    pub fn from_path(path: &Path) -> Option<Self> {
        let extension = path.extension()?.to_str()?;
        Self::from_extension(extension)
    }

    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension
            .trim_start_matches('.')
            .to_ascii_lowercase()
            .as_str()
        {
            "xls" => Some(Self::Xls),
            "xlsx" => Some(Self::Xlsx),
            "xlsm" => Some(Self::Xlsm),
            "xlsb" => Some(Self::Xlsb),
            "xltx" => Some(Self::Xltx),
            "xltm" => Some(Self::Xltm),
            "ods" => Some(Self::Ods),
            "csv" => Some(Self::Csv),
            "tsv" | "tab" => Some(Self::Tsv),
            _ => None,
        }
    }

    pub fn from_content_type(content_type: &str) -> Option<Self> {
        let media_type = content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim()
            .to_ascii_lowercase();
        match media_type.as_str() {
            "application/vnd.ms-excel" => Some(Self::Xls),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => Some(Self::Xlsx),
            "application/vnd.ms-excel.sheet.macroenabled.12" => Some(Self::Xlsm),
            "application/vnd.ms-excel.sheet.binary.macroenabled.12" => Some(Self::Xlsb),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template" => {
                Some(Self::Xltx)
            }
            "application/vnd.ms-excel.template.macroenabled.12" => Some(Self::Xltm),
            "application/vnd.oasis.opendocument.spreadsheet" => Some(Self::Ods),
            "text/csv" => Some(Self::Csv),
            "text/tab-separated-values" => Some(Self::Tsv),
            _ => None,
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Self::Xls => "xls",
            Self::Xlsx => "xlsx",
            Self::Xlsm => "xlsm",
            Self::Xlsb => "xlsb",
            Self::Xltx => "xltx",
            Self::Xltm => "xltm",
            Self::Ods => "ods",
            Self::Csv => "csv",
            Self::Tsv => "tsv",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            Self::Xls => "application/vnd.ms-excel",
            Self::Xlsx => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            Self::Xlsm => "application/vnd.ms-excel.sheet.macroEnabled.12",
            Self::Xlsb => "application/vnd.ms-excel.sheet.binary.macroEnabled.12",
            Self::Xltx => "application/vnd.openxmlformats-officedocument.spreadsheetml.template",
            Self::Xltm => "application/vnd.ms-excel.template.macroEnabled.12",
            Self::Ods => "application/vnd.oasis.opendocument.spreadsheet",
            Self::Csv => "text/csv; charset=utf-8",
            Self::Tsv => "text/tab-separated-values; charset=utf-8",
        }
    }

    pub fn is_delimited(self) -> bool {
        matches!(self, Self::Csv | Self::Tsv)
    }

    pub fn is_workbook(self) -> bool {
        !self.is_delimited()
    }

    pub fn is_ooxml(self) -> bool {
        matches!(self, Self::Xlsx | Self::Xlsm | Self::Xltx | Self::Xltm)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_supported_extensions_case_insensitively() {
        assert_eq!(
            SpreadsheetFileFormat::from_path(Path::new("legacy.XLS")),
            Some(SpreadsheetFileFormat::Xls)
        );
        assert_eq!(
            SpreadsheetFileFormat::from_path(Path::new("data.tab")),
            Some(SpreadsheetFileFormat::Tsv)
        );
        assert_eq!(
            SpreadsheetFileFormat::from_path(Path::new("data.bin")),
            None
        );
    }
}
