use super::acl_transaction::NamedAclMutex;
use super::process_launch::{last_error, wide};
use anyhow::{Context, Result};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::io::Write;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

const LEGACY_ACL_LEDGER_VERSION: u32 = 1;
const ACL_LEDGER_VERSION: u32 = 2;
const LEGACY_ACL_LEDGER_FILE: &str = "acl-ledger.json";
const ACL_LEDGER_FILE: &str = "acl-ledger.v2.json";
const ACL_LEDGER_MIGRATION_MARKER: &str = "acl-ledger.v2.migrated.json";
const LEGACY_ACL_LEDGER_BACKUP: &str = "acl-ledger.v1.pre-v2.json";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum PersistentAclKind {
    Read,
    ManagedRuntimeRead,
    ManagedRuntimeTraverse,
    Write,
    DenyRead,
    DenyWrite,
    /// Preserve extensions written by a newer helper. Ordinary execution can
    /// continue, while destructive cleanup leaves semantics it does not
    /// understand for the owning helper version.
    Unknown(String),
}

impl PersistentAclKind {
    fn as_str(&self) -> &str {
        match self {
            Self::Read => "read",
            Self::ManagedRuntimeRead => "managed_runtime_read",
            Self::ManagedRuntimeTraverse => "managed_runtime_traverse",
            Self::Write => "write",
            Self::DenyRead => "deny_read",
            Self::DenyWrite => "deny_write",
            Self::Unknown(value) => value,
        }
    }

    fn is_legacy_compatible(&self) -> bool {
        matches!(
            self,
            Self::Read | Self::Write | Self::DenyRead | Self::DenyWrite
        )
    }

    pub(super) fn is_unknown(&self) -> bool {
        matches!(self, Self::Unknown(_))
    }
}

impl Serialize for PersistentAclKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for PersistentAclKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Ok(match value.as_str() {
            "read" => Self::Read,
            "managed_runtime_read" => Self::ManagedRuntimeRead,
            "managed_runtime_traverse" => Self::ManagedRuntimeTraverse,
            "write" => Self::Write,
            "deny_read" => Self::DenyRead,
            "deny_write" => Self::DenyWrite,
            _ => Self::Unknown(value),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub(super) struct PersistentAclEntry {
    pub(super) account: String,
    pub(super) path: PathBuf,
    pub(super) kind: PersistentAclKind,
    #[serde(default)]
    pub(super) sid: Vec<u8>,
    #[serde(default = "legacy_acl_entry_permissions_version")]
    pub(super) permissions_version: u32,
    #[serde(default)]
    pub(super) object_generation: String,
}

fn legacy_acl_entry_permissions_version() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct PersistentAclLedger {
    version: u32,
    pub(super) entries: Vec<PersistentAclEntry>,
}

impl Default for PersistentAclLedger {
    fn default() -> Self {
        Self {
            version: ACL_LEDGER_VERSION,
            entries: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
struct LegacyLedgerFingerprint {
    length: u64,
    modified_nanos: u128,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerMigrationMarker {
    ledger_version: u32,
    legacy: Option<LegacyLedgerFingerprint>,
}

struct AclLedgerPaths {
    legacy: PathBuf,
    current: PathBuf,
    marker: PathBuf,
    backup: PathBuf,
}

impl AclLedgerPaths {
    fn current() -> Self {
        Self::for_state_dir(&crate::setup::state_dir())
    }

    fn for_state_dir(state_dir: &Path) -> Self {
        Self {
            legacy: state_dir.join(LEGACY_ACL_LEDGER_FILE),
            current: state_dir.join(ACL_LEDGER_FILE),
            marker: state_dir.join(ACL_LEDGER_MIGRATION_MARKER),
            backup: state_dir.join(LEGACY_ACL_LEDGER_BACKUP),
        }
    }
}

pub(super) fn load_acl_ledger() -> Result<PersistentAclLedger> {
    let paths = AclLedgerPaths::current();
    if layout_requires_sync(&paths)? {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        if layout_requires_sync(&paths)? {
            synchronize_ledger_layout(&paths)?;
        }
    }
    read_ledger(&paths.current, ACL_LEDGER_VERSION)
}

pub(super) fn save_acl_ledger(ledger: &PersistentAclLedger) -> Result<()> {
    anyhow::ensure!(
        ledger.version == ACL_LEDGER_VERSION,
        "refusing to save ACL ledger version {} as version {}",
        ledger.version,
        ACL_LEDGER_VERSION
    );
    let paths = AclLedgerPaths::current();
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    // Publish the legacy-safe projection first. If the process stops before
    // v2 or the marker is updated, the fingerprint mismatch makes the next
    // v2 load reconcile this authoritative compatibility projection.
    write_legacy_projection(&paths, ledger)?;
    write_json_atomically(&paths.current, ledger)?;
    write_migration_marker(&paths)
}

fn layout_requires_sync(paths: &AclLedgerPaths) -> Result<bool> {
    if !paths.current.is_file() || !paths.marker.is_file() {
        return Ok(true);
    }
    let marker: LedgerMigrationMarker =
        serde_json::from_slice(&std::fs::read(&paths.marker).with_context(|| {
            format!(
                "read ACL ledger migration marker {}",
                paths.marker.display()
            )
        })?)
        .with_context(|| {
            format!(
                "parse ACL ledger migration marker {}",
                paths.marker.display()
            )
        })?;
    Ok(marker.ledger_version != ACL_LEDGER_VERSION
        || marker.legacy != legacy_fingerprint(&paths.legacy)?)
}

fn synchronize_ledger_layout(paths: &AclLedgerPaths) -> Result<()> {
    let legacy = if paths.legacy.is_file() {
        Some(read_ledger(&paths.legacy, LEGACY_ACL_LEDGER_VERSION)?)
    } else {
        None
    };
    let mut current = if paths.current.is_file() {
        read_ledger(&paths.current, ACL_LEDGER_VERSION)?
    } else {
        PersistentAclLedger::default()
    };

    if let Some(legacy) = legacy.as_ref() {
        backup_legacy_ledger(paths)?;
        // A v1 helper drops fields it does not know (notably the v2 object
        // generation) whenever it rewrites the file. Keep the richer v2
        // record for an existing slot and absorb only genuinely new v1 slots.
        // Actual ACE verification remains authoritative for removals.
        for entry in &legacy.entries {
            if !current
                .entries
                .iter()
                .any(|existing| same_slot(existing, entry))
            {
                current.entries.push(entry.clone());
            }
        }
        write_json_atomically(&paths.current, &current)?;
        write_legacy_projection(paths, &current)?;
    } else {
        write_json_atomically(&paths.current, &current)?;
    }
    write_migration_marker(paths)
}

fn write_legacy_projection(paths: &AclLedgerPaths, current: &PersistentAclLedger) -> Result<()> {
    let compatible = PersistentAclLedger {
        version: LEGACY_ACL_LEDGER_VERSION,
        entries: current
            .entries
            .iter()
            .filter(|entry| entry.kind.is_legacy_compatible())
            .cloned()
            .collect(),
    };
    write_json_atomically_if_changed(&paths.legacy, &compatible)
}

fn write_migration_marker(paths: &AclLedgerPaths) -> Result<()> {
    let marker = LedgerMigrationMarker {
        ledger_version: ACL_LEDGER_VERSION,
        legacy: legacy_fingerprint(&paths.legacy)?,
    };
    write_json_atomically_if_changed(&paths.marker, &marker)
}

fn same_slot(left: &PersistentAclEntry, right: &PersistentAclEntry) -> bool {
    left.account.eq_ignore_ascii_case(&right.account)
        && left.path == right.path
        && left.kind == right.kind
}

fn read_ledger(path: &Path, expected_version: u32) -> Result<PersistentAclLedger> {
    if !path.exists() && expected_version == ACL_LEDGER_VERSION {
        return Ok(PersistentAclLedger::default());
    }
    let mut ledger: PersistentAclLedger = serde_json::from_slice(
        &std::fs::read(path).with_context(|| format!("read ACL ledger {}", path.display()))?,
    )
    .with_context(|| format!("parse ACL ledger {}", path.display()))?;
    anyhow::ensure!(
        ledger.version == expected_version,
        "unsupported ACL ledger version {} in {} (expected {})",
        ledger.version,
        path.display(),
        expected_version
    );
    ledger.entries.retain(|entry| entry.path.exists());
    Ok(ledger)
}

fn backup_legacy_ledger(paths: &AclLedgerPaths) -> Result<()> {
    if paths.backup.exists() {
        return Ok(());
    }
    crate::setup::ensure_parent(&paths.backup)?;
    let bytes = std::fs::read(&paths.legacy)
        .with_context(|| format!("read legacy ACL ledger {}", paths.legacy.display()))?;
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&paths.backup)
    {
        Ok(mut file) => {
            file.write_all(&bytes)
                .with_context(|| format!("backup legacy ACL ledger {}", paths.backup.display()))?;
            file.sync_all().with_context(|| {
                format!("flush legacy ACL ledger backup {}", paths.backup.display())
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => {
            return Err(error).with_context(|| {
                format!("create legacy ACL ledger backup {}", paths.backup.display())
            })
        }
    }
    Ok(())
}

fn legacy_fingerprint(path: &Path) -> Result<Option<LegacyLedgerFingerprint>> {
    let metadata = match std::fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("inspect legacy ACL ledger {}", path.display()))
        }
    };
    let modified_nanos = metadata
        .modified()
        .ok()
        .and_then(|value| value.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|value| value.as_nanos())
        .unwrap_or_default();
    Ok(Some(LegacyLedgerFingerprint {
        length: metadata.len(),
        modified_nanos,
    }))
}

fn write_json_atomically(path: &Path, value: &impl Serialize) -> Result<()> {
    write_bytes_atomically(path, &serde_json::to_vec_pretty(value)?)
}

fn write_json_atomically_if_changed(path: &Path, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if std::fs::read(path).is_ok_and(|current| current == bytes) {
        return Ok(());
    }
    write_bytes_atomically(path, &bytes)
}

fn write_bytes_atomically(path: &Path, bytes: &[u8]) -> Result<()> {
    crate::setup::ensure_parent(path)?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, bytes)
        .with_context(|| format!("write ACL ledger temporary file {}", temporary.display()))?;
    let temporary_w = wide(temporary.as_os_str());
    let path_w = wide(path.as_os_str());
    let moved = unsafe {
        MoveFileExW(
            temporary_w.as_ptr(),
            path_w.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if moved == 0 {
        let _ = std::fs::remove_file(&temporary);
        return Err(last_error("publish ACL ledger with MoveFileExW"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        layout_requires_sync, read_ledger, synchronize_ledger_layout, AclLedgerPaths,
        PersistentAclEntry, PersistentAclKind, PersistentAclLedger, ACL_LEDGER_VERSION,
        LEGACY_ACL_LEDGER_VERSION,
    };
    use std::path::PathBuf;
    use uuid::Uuid;

    #[derive(serde::Deserialize)]
    struct LegacyReaderLedger {
        version: u32,
        entries: Vec<LegacyReaderEntry>,
    }

    #[derive(serde::Deserialize)]
    struct LegacyReaderEntry {
        kind: LegacyReaderKind,
    }

    #[derive(Debug, serde::Deserialize, PartialEq, Eq)]
    #[serde(rename_all = "snake_case")]
    enum LegacyReaderKind {
        Read,
        Write,
        DenyRead,
        DenyWrite,
    }

    #[test]
    fn migration_preserves_new_entries_and_leaves_v1_readable_by_old_helpers() {
        let state =
            std::env::temp_dir().join(format!("opentopia-acl-ledger-migration-{}", Uuid::new_v4()));
        std::fs::create_dir_all(&state).expect("create ledger migration fixture");
        let existing = state.join("existing");
        std::fs::create_dir(&existing).expect("create ledger entry path");
        let paths = AclLedgerPaths::for_state_dir(&state);
        let legacy = PersistentAclLedger {
            version: LEGACY_ACL_LEDGER_VERSION,
            entries: vec![
                entry(&existing, PersistentAclKind::Write),
                entry(&existing, PersistentAclKind::ManagedRuntimeTraverse),
                entry(&existing, PersistentAclKind::Unknown("future_kind".into())),
            ],
        };
        super::write_json_atomically(&paths.legacy, &legacy).expect("write legacy ledger");

        synchronize_ledger_layout(&paths).expect("migrate ledger layout");

        let current = read_ledger(&paths.current, ACL_LEDGER_VERSION).expect("read v2 ledger");
        assert_eq!(current.entries.len(), 3);
        assert!(current
            .entries
            .iter()
            .any(|entry| entry.kind == PersistentAclKind::ManagedRuntimeTraverse));
        assert!(current.entries.iter().any(|entry| entry.kind.is_unknown()));
        let legacy = read_ledger(&paths.legacy, LEGACY_ACL_LEDGER_VERSION)
            .expect("read sanitized v1 ledger");
        assert_eq!(legacy.entries.len(), 1);
        assert_eq!(legacy.entries[0].kind, PersistentAclKind::Write);
        let old_reader: LegacyReaderLedger = serde_json::from_slice(
            &std::fs::read(&paths.legacy).expect("read v1 ledger for old helper"),
        )
        .expect("old helper schema must parse sanitized v1 ledger");
        assert_eq!(old_reader.version, LEGACY_ACL_LEDGER_VERSION);
        assert_eq!(old_reader.entries.len(), 1);
        assert_eq!(old_reader.entries[0].kind, LegacyReaderKind::Write);
        assert!(paths.backup.is_file());
        assert!(!layout_requires_sync(&paths).expect("inspect synchronized layout"));

        let later_path = state.join("written-by-old-helper");
        std::fs::create_dir(&later_path).expect("create later legacy entry path");
        let mut legacy = legacy;
        for entry in &mut legacy.entries {
            entry.object_generation.clear();
        }
        let mut later_entry = entry(&later_path, PersistentAclKind::Read);
        later_entry.object_generation.clear();
        legacy.entries.push(later_entry);
        super::write_json_atomically(&paths.legacy, &legacy)
            .expect("simulate a later v1 helper write");
        assert!(layout_requires_sync(&paths).expect("detect later v1 write"));
        synchronize_ledger_layout(&paths).expect("merge later v1 write");
        let current = read_ledger(&paths.current, ACL_LEDGER_VERSION)
            .expect("read v2 ledger after coexistence merge");
        assert!(current
            .entries
            .iter()
            .any(|entry| entry.path == later_path && entry.kind == PersistentAclKind::Read));
        assert!(current.entries.iter().any(|entry| {
            entry.path == existing
                && entry.kind == PersistentAclKind::Write
                && entry.object_generation == "generation"
        }));

        std::fs::remove_dir_all(state).expect("remove ledger migration fixture");
    }

    #[test]
    fn unknown_acl_kinds_round_trip_without_data_loss() {
        let encoded = serde_json::to_string(&PersistentAclKind::Unknown("future_read".into()))
            .expect("serialize unknown ACL kind");
        assert_eq!(encoded, "\"future_read\"");
        let decoded: PersistentAclKind =
            serde_json::from_str(&encoded).expect("deserialize unknown ACL kind");
        assert_eq!(decoded, PersistentAclKind::Unknown("future_read".into()));
    }

    fn entry(path: &std::path::Path, kind: PersistentAclKind) -> PersistentAclEntry {
        PersistentAclEntry {
            account: "OpenTopiaSandboxUsers".into(),
            path: PathBuf::from(path),
            kind,
            sid: vec![1, 2, 3],
            permissions_version: 2,
            object_generation: "generation".into(),
        }
    }
}
