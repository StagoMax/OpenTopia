use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};
use thiserror::Error;

pub const DEFAULT_MAX_SOURCE_FILES: usize = 20;
pub const DEFAULT_MAX_SOURCE_BYTES: u64 = 25 * 1024 * 1024;
pub const DEFAULT_MAX_TEXT_BYTES: usize = 256 * 1024;
pub const DEFAULT_MAX_TOTAL_TEXT_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ContextSourceKind {
    Text,
    Image,
    Document,
}

#[derive(Debug, Clone)]
pub struct ContextSourcePolicy {
    pub max_files: usize,
    pub max_file_bytes: u64,
    pub max_text_bytes: usize,
    pub max_total_text_bytes: usize,
}

impl Default for ContextSourcePolicy {
    fn default() -> Self {
        Self {
            max_files: DEFAULT_MAX_SOURCE_FILES,
            max_file_bytes: DEFAULT_MAX_SOURCE_BYTES,
            max_text_bytes: DEFAULT_MAX_TEXT_BYTES,
            max_total_text_bytes: DEFAULT_MAX_TOTAL_TEXT_BYTES,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LoadedContextSource {
    pub path: PathBuf,
    pub name: String,
    pub kind: ContextSourceKind,
    pub content_type: String,
    pub bytes: u64,
    pub text: Option<String>,
    pub truncated: bool,
}

impl LoadedContextSource {
    pub fn render_for_model(&self) -> String {
        let header = format!(
            "Source: {}\nPath: {}\nType: {}\nBytes: {}",
            self.name,
            self.path.display(),
            self.content_type,
            self.bytes
        );
        match &self.text {
            Some(text) => format!(
                "<source>\n{header}\nTruncated: {}\n\n{text}\n</source>",
                self.truncated
            ),
            None => format!(
                "<source>\n{header}\nContent: binary content is referenced but not embedded\n</source>"
            ),
        }
    }
}

#[derive(Debug, Error)]
pub enum ContextSourceError {
    #[error("too many sources: {actual}; maximum is {maximum}")]
    TooManySources { actual: usize, maximum: usize },
    #[error("source does not exist or cannot be resolved: {path}")]
    NotFound { path: String },
    #[error("source is not a regular file: {path}")]
    NotAFile { path: String },
    #[error("source metadata cannot be read: {path}")]
    Metadata { path: String },
    #[error("source is too large: {path} is {bytes} bytes; maximum is {maximum}")]
    TooLarge {
        path: String,
        bytes: u64,
        maximum: u64,
    },
    #[error("sensitive source is not allowed: {path}")]
    Sensitive { path: String },
    #[error("unsupported source type: {path}")]
    Unsupported { path: String },
    #[error("source cannot be read: {path}")]
    Read { path: String },
}

pub fn load_context_sources(
    paths: &[PathBuf],
    policy: &ContextSourcePolicy,
) -> Result<Vec<LoadedContextSource>, ContextSourceError> {
    if paths.len() > policy.max_files {
        return Err(ContextSourceError::TooManySources {
            actual: paths.len(),
            maximum: policy.max_files,
        });
    }

    let mut seen = HashSet::new();
    let mut sources = Vec::new();
    let mut remaining_text_bytes = policy.max_total_text_bytes;
    for path in paths {
        let canonical = path
            .canonicalize()
            .map_err(|_| ContextSourceError::NotFound {
                path: path.display().to_string(),
            })?;
        let key = canonical_path_key(&canonical);
        if !seen.insert(key) {
            continue;
        }
        let metadata = canonical
            .metadata()
            .map_err(|_| ContextSourceError::Metadata {
                path: canonical.display().to_string(),
            })?;
        if !metadata.is_file() {
            return Err(ContextSourceError::NotAFile {
                path: canonical.display().to_string(),
            });
        }
        if metadata.len() > policy.max_file_bytes {
            return Err(ContextSourceError::TooLarge {
                path: canonical.display().to_string(),
                bytes: metadata.len(),
                maximum: policy.max_file_bytes,
            });
        }
        if is_sensitive_source(&canonical) {
            return Err(ContextSourceError::Sensitive {
                path: canonical.display().to_string(),
            });
        }

        let (kind, content_type) =
            classify_source(&canonical).ok_or_else(|| ContextSourceError::Unsupported {
                path: canonical.display().to_string(),
            })?;
        let name = canonical
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("source")
            .to_string();
        let (text, truncated) = if kind == ContextSourceKind::Text {
            let limit = policy.max_text_bytes.min(remaining_text_bytes);
            let (text, truncated) = read_text_limited(&canonical, limit, metadata.len())?;
            remaining_text_bytes = remaining_text_bytes.saturating_sub(text.len());
            (Some(text), truncated)
        } else {
            (None, false)
        };
        sources.push(LoadedContextSource {
            path: canonical,
            name,
            kind,
            content_type: content_type.to_string(),
            bytes: metadata.len(),
            text,
            truncated,
        });
    }
    Ok(sources)
}

fn read_text_limited(
    path: &Path,
    limit: usize,
    original_bytes: u64,
) -> Result<(String, bool), ContextSourceError> {
    if limit == 0 {
        return Ok((String::new(), original_bytes > 0));
    }
    let mut file = File::open(path).map_err(|_| ContextSourceError::Read {
        path: path.display().to_string(),
    })?;
    let mut bytes = Vec::with_capacity(limit.min(64 * 1024));
    file.by_ref()
        .take(limit as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ContextSourceError::Read {
            path: path.display().to_string(),
        })?;
    Ok((
        String::from_utf8_lossy(&bytes).into_owned(),
        original_bytes > bytes.len() as u64,
    ))
}

fn classify_source(path: &Path) -> Option<(ContextSourceKind, &'static str)> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
    let value = match extension.as_str() {
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" | "c" | "h" | "cc" | "cpp" | "hpp"
        | "py" | "go" | "java" | "kt" | "swift" | "rb" | "php" | "sh" | "ps1" | "bat" | "cmd"
        | "sql" | "graphql" | "gql" | "proto" | "diff" | "patch" => {
            (ContextSourceKind::Text, "text/plain; charset=utf-8")
        }
        "json" | "jsonc" | "jsonl" => (ContextSourceKind::Text, "application/json"),
        "yaml" | "yml" => (ContextSourceKind::Text, "application/yaml"),
        "toml" => (ContextSourceKind::Text, "application/toml"),
        "xml" => (ContextSourceKind::Text, "application/xml"),
        "html" | "htm" => (ContextSourceKind::Text, "text/html"),
        "css" | "scss" | "less" => (ContextSourceKind::Text, "text/css"),
        "md" | "mdx" => (ContextSourceKind::Text, "text/markdown"),
        "txt" | "log" | "csv" | "tsv" | "ini" | "conf" | "config" | "properties" => {
            (ContextSourceKind::Text, "text/plain; charset=utf-8")
        }
        "png" => (ContextSourceKind::Image, "image/png"),
        "jpg" | "jpeg" => (ContextSourceKind::Image, "image/jpeg"),
        "gif" => (ContextSourceKind::Image, "image/gif"),
        "webp" => (ContextSourceKind::Image, "image/webp"),
        "bmp" => (ContextSourceKind::Image, "image/bmp"),
        "pdf" => (ContextSourceKind::Document, "application/pdf"),
        "docx" => (
            ContextSourceKind::Document,
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        ),
        "xlsx" => (
            ContextSourceKind::Document,
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        ),
        "pptx" => (
            ContextSourceKind::Document,
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ),
        _ => return None,
    };
    Some(value)
}

fn is_sensitive_source(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase();
    name == ".env"
        || name.starts_with(".env.")
        || matches!(
            name.as_str(),
            ".npmrc"
                | ".pypirc"
                | ".netrc"
                | "credentials"
                | "credentials.json"
                | "secrets.json"
                | "id_rsa"
                | "id_ed25519"
        )
        || matches!(
            extension.as_str(),
            "pem" | "key" | "p12" | "pfx" | "keystore" | "jks" | "der"
        )
}

fn canonical_path_key(path: &Path) -> String {
    let value = path.to_string_lossy().replace('\\', "/");
    if cfg!(windows) {
        value.to_ascii_lowercase()
    } else {
        value
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use uuid::Uuid;

    struct TestDir(PathBuf);

    impl TestDir {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!("opentopia-sources-{}", Uuid::new_v4()));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }

        fn write(&self, name: &str, content: &[u8]) -> PathBuf {
            let path = self.0.join(name);
            fs::write(&path, content).unwrap();
            path
        }
    }

    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn loads_text_and_deduplicates_canonical_paths() {
        let dir = TestDir::new();
        let path = dir.write("main.rs", b"fn main() {}\n");
        let sources =
            load_context_sources(&[path.clone(), path], &ContextSourcePolicy::default()).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].kind, ContextSourceKind::Text);
        assert_eq!(sources[0].text.as_deref(), Some("fn main() {}\n"));
        assert!(!sources[0].truncated);
    }

    #[test]
    fn truncates_text_at_per_file_and_total_limits() {
        let dir = TestDir::new();
        let first = dir.write("one.txt", b"1234567890");
        let second = dir.write("two.txt", b"abcdefghij");
        let policy = ContextSourcePolicy {
            max_text_bytes: 8,
            max_total_text_bytes: 10,
            ..ContextSourcePolicy::default()
        };
        let sources = load_context_sources(&[first, second], &policy).unwrap();
        assert_eq!(sources[0].text.as_deref(), Some("12345678"));
        assert_eq!(sources[1].text.as_deref(), Some("ab"));
        assert!(sources.iter().all(|source| source.truncated));
    }

    #[test]
    fn image_and_document_content_are_not_embedded() {
        let dir = TestDir::new();
        let image = dir.write("screen.png", b"not-a-real-image");
        let document = dir.write("spec.pdf", b"not-a-real-pdf");
        let sources =
            load_context_sources(&[image, document], &ContextSourcePolicy::default()).unwrap();
        assert_eq!(sources[0].kind, ContextSourceKind::Image);
        assert_eq!(sources[1].kind, ContextSourceKind::Document);
        assert!(sources.iter().all(|source| source.text.is_none()));
    }

    #[test]
    fn rejects_sensitive_unknown_large_missing_and_directory_sources() {
        let dir = TestDir::new();
        let sensitive = dir.write(".env", b"TOKEN=secret");
        assert!(matches!(
            load_context_sources(&[sensitive], &ContextSourcePolicy::default()),
            Err(ContextSourceError::Sensitive { .. })
        ));

        let unknown = dir.write("archive.bin", b"binary");
        assert!(matches!(
            load_context_sources(&[unknown], &ContextSourcePolicy::default()),
            Err(ContextSourceError::Unsupported { .. })
        ));

        let large = dir.write("large.txt", b"12345");
        let policy = ContextSourcePolicy {
            max_file_bytes: 4,
            ..ContextSourcePolicy::default()
        };
        assert!(matches!(
            load_context_sources(&[large], &policy),
            Err(ContextSourceError::TooLarge { .. })
        ));

        assert!(matches!(
            load_context_sources(
                &[dir.0.join("missing.txt")],
                &ContextSourcePolicy::default()
            ),
            Err(ContextSourceError::NotFound { .. })
        ));
        assert!(matches!(
            load_context_sources(
                std::slice::from_ref(&dir.0),
                &ContextSourcePolicy::default()
            ),
            Err(ContextSourceError::NotAFile { .. })
        ));
    }

    #[test]
    fn enforces_batch_limit_and_accepts_explicit_file_outside_a_workspace() {
        let dir = TestDir::new();
        let path = dir.write("outside.md", b"selected explicitly");
        let policy = ContextSourcePolicy {
            max_files: 0,
            ..ContextSourcePolicy::default()
        };
        assert!(matches!(
            load_context_sources(std::slice::from_ref(&path), &policy),
            Err(ContextSourceError::TooManySources { .. })
        ));
        let source = load_context_sources(&[path], &ContextSourcePolicy::default())
            .unwrap()
            .remove(0);
        assert!(source.path.is_absolute());
    }
}
