use super::*;
use rust_xlsxwriter::{Color, Format};
use std::fs::OpenOptions;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

struct TestDirectory(PathBuf);

impl TestDirectory {
    fn new() -> Self {
        let unique = format!(
            "opentopia-spreadsheet-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system time")
                .as_nanos(),
            TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        );
        let path = std::env::temp_dir().join(unique);
        fs::create_dir_all(&path).expect("create test directory");
        Self(path)
    }

    fn path(&self, file_name: &str) -> PathBuf {
        self.0.join(file_name)
    }
}

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn address(row: u32, column: u32) -> CellAddress {
    CellAddress { row, column }
}

fn range(start: (u32, u32), end: (u32, u32)) -> CellRange {
    CellRange {
        start: address(start.0, start.1),
        end: address(end.0, end.1),
    }
}

fn update(row: u32, column: u32, value: SpreadsheetCellInput) -> CellUpdate {
    CellUpdate {
        address: address(row, column),
        value,
        style_from: None,
    }
}

fn zip_part(path: &Path, name: &str) -> Vec<u8> {
    let bytes = fs::read(path).expect("read XLSX package");
    let mut archive = ZipArchive::new(Cursor::new(bytes)).expect("open XLSX package");
    let mut part = archive.by_name(name).expect("open XLSX part");
    let mut contents = Vec::new();
    part.read_to_end(&mut contents).expect("read XLSX part");
    contents
}

mod read_validation;
mod write_roundtrip;
