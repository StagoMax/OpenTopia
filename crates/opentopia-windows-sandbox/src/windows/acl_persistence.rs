use super::acl_transaction::{
    propagate_inherited_dacl, update_dacl, AclTransaction, NamedAclMutex,
};
use super::process_launch::{last_error, wide};
use super::security_token::{effective_file_access, SidBuffer};
use super::{
    broker_exchange_root, normalized_capability_path, SandboxRequest,
    ACL_ENTRY_PERMISSIONS_VERSION, CAPABILITY_NAMESPACE, CAPABILITY_PRINCIPAL_PREFIX,
    LEGACY_CAPABILITY_PRINCIPAL, LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX,
    WORKSPACE_WRITE_PERMISSIONS,
};
use anyhow::{Context, Result};
use opentopia_sandbox_protocol::ReadProvisioning;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::HANDLE;
use windows_sys::Win32::Security::Authorization::REVOKE_ACCESS;
use windows_sys::Win32::Security::{LookupAccountNameW, PSID};
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, MOVEFILE_REPLACE_EXISTING,
    MOVEFILE_WRITE_THROUGH,
};

pub(super) fn ensure_broker_exchange_permissions(
    account: &str,
    sid: PSID,
    token: HANDLE,
) -> Result<()> {
    let path = broker_exchange_root(account);
    std::fs::create_dir_all(&path)
        .with_context(|| format!("create broker exchange root {}", path.display()))?;
    if effective_file_access(token, &path, WORKSPACE_WRITE_PERMISSIONS)? {
        return Ok(());
    }
    let _guards = NamedAclMutex::acquire_paths([path.as_path()])?;
    let mut transaction = AclTransaction::default();
    transaction.grant(&path, sid, true, WORKSPACE_WRITE_PERMISSIONS)?;
    let entry = PersistentAclEntry {
        account: account.to_string(),
        path,
        kind: PersistentAclKind::Write,
        sid: SidBuffer::copy_from_sid(sid)?.0,
        permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
    };
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut ledger = load_acl_ledger()?;
    ledger.entries.retain(|existing| {
        existing.account != entry.account
            || existing.path != entry.path
            || existing.kind != entry.kind
    });
    ledger.entries.push(entry);
    save_acl_ledger(&ledger)?;
    transaction.commit();
    Ok(())
}

const ACL_LEDGER_VERSION: u32 = 1;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum PersistentAclKind {
    Read,
    Write,
    DenyRead,
    DenyWrite,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct PersistentAclEntry {
    account: String,
    path: std::path::PathBuf,
    kind: PersistentAclKind,
    #[serde(default)]
    sid: Vec<u8>,
    #[serde(default = "legacy_acl_entry_permissions_version")]
    permissions_version: u32,
}

fn legacy_acl_entry_permissions_version() -> u32 {
    1
}

#[derive(Debug, serde::Serialize, serde::Deserialize)]
struct PersistentAclLedger {
    version: u32,
    entries: Vec<PersistentAclEntry>,
}

impl Default for PersistentAclLedger {
    fn default() -> Self {
        Self {
            version: ACL_LEDGER_VERSION,
            entries: Vec::new(),
        }
    }
}

fn acl_ledger_path() -> std::path::PathBuf {
    crate::setup::state_dir().join("acl-ledger.json")
}

fn load_acl_ledger() -> Result<PersistentAclLedger> {
    let path = acl_ledger_path();
    if !path.exists() {
        return Ok(PersistentAclLedger::default());
    }
    let mut ledger: PersistentAclLedger = serde_json::from_slice(
        &std::fs::read(&path).with_context(|| format!("read ACL ledger {}", path.display()))?,
    )
    .with_context(|| format!("parse ACL ledger {}", path.display()))?;
    if ledger.version != ACL_LEDGER_VERSION {
        anyhow::bail!(
            "unsupported ACL ledger version {} (expected {})",
            ledger.version,
            ACL_LEDGER_VERSION
        )
    }
    ledger.entries.retain(|entry| entry.path.exists());
    Ok(ledger)
}

fn save_acl_ledger(ledger: &PersistentAclLedger) -> Result<()> {
    let path = acl_ledger_path();
    crate::setup::ensure_parent(&path)?;
    let temporary = path.with_extension(format!("json.{}.tmp", std::process::id()));
    std::fs::write(&temporary, serde_json::to_vec_pretty(ledger)?)
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

pub(super) fn verify_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    token: HANDLE,
) -> Result<()> {
    let mut missing = Vec::new();
    for path in request
        .filesystem
        .deny_read
        .iter()
        .filter(|path| path.exists())
    {
        if effective_file_access(token, path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)? {
            missing.push(format!("deny_read:{}", path.display()));
        }
    }
    for capability in &request.filesystem.read_execute {
        if request
            .filesystem
            .write
            .iter()
            .any(|write_root| path_starts_with(&capability.path, write_root))
        {
            continue;
        }
        if !effective_file_access(
            token,
            &capability.path,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )? {
            let kind = match capability.provisioning {
                ReadProvisioning::Managed => "managed_read",
                ReadProvisioning::ExistingOnly => "external_runtime",
            };
            missing.push(format!("{kind}:{}", capability.path.display()));
        }
    }
    for path in &request.filesystem.write {
        if !effective_file_access(token, path, WORKSPACE_WRITE_PERMISSIONS)? {
            missing.push(format!("managed_write:{}", path.display()));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stage=provision_acl sandbox account '{account}' is not prepared for this policy scope: {}. Run `opentopia-sandbox provision` for managed roots; external_runtime roots are immutable and must already allow normal-user read/execute access",
        missing.join(", ")
    )
}

pub(super) fn verify_persistent_capability_permissions(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<()> {
    let desired = capability_acl_entries(request, principal, sid)?;
    let ledger = load_acl_ledger()?;
    let missing = desired
        .iter()
        .filter(|entry| !ledger.entries.contains(entry))
        .map(|entry| format!("{:?}:{}", entry.kind, entry.path.display()))
        .collect::<Vec<_>>();
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stage=provision_acl capability scope '{principal}' is not prepared: {}. Run `opentopia-sandbox provision` before command startup",
        missing.join(", ")
    )
}

pub(super) fn ensure_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    sid: PSID,
    write_account: &str,
    write_sid: PSID,
    token: HANDLE,
) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let mut desired = Vec::new();
    for path in request
        .filesystem
        .deny_read
        .iter()
        .filter(|path| path.exists())
    {
        let deny_entry_installed = ledger.entries.iter().any(|entry| {
            entry.account.eq_ignore_ascii_case(account)
                && entry.path == *path
                && entry.kind == PersistentAclKind::DenyRead
        });
        if !deny_entry_installed
            && effective_file_access(token, path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?
        {
            desired.push((path.clone(), PersistentAclKind::DenyRead));
        }
    }

    let mut inaccessible_external = Vec::new();
    for capability in &request.filesystem.read_execute {
        if request
            .filesystem
            .write
            .iter()
            .any(|write_root| path_starts_with(&capability.path, write_root))
        {
            continue;
        }
        if effective_file_access(
            token,
            &capability.path,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )? {
            crate::logging::event(
                "access_check",
                format!(
                    "satisfied account={account} intent=read_execute path={}",
                    capability.path.display()
                ),
            );
            continue;
        }
        // ExistingOnly is an immutable boundary even when a broad managed
        // root happens to contain it. External SDK/runtime ACLs are never
        // rewritten by OpenTopia.
        match capability.provisioning {
            ReadProvisioning::Managed => {
                desired.push((capability.path.clone(), PersistentAclKind::Read))
            }
            ReadProvisioning::ExistingOnly => inaccessible_external.push(capability.path.clone()),
        }
    }
    if !inaccessible_external.is_empty() {
        let paths = inaccessible_external
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        anyhow::bail!(
            "stage=resolve_runtime runtime_not_accessible sandbox account '{account}' cannot read/execute external runtime roots: {paths}. OpenTopia did not rewrite their host ACLs; provision the runtime for normal-user read access or place it in an OpenTopia-managed runtime location"
        )
    }

    for path in &request.filesystem.write {
        if !effective_file_access(token, path, WORKSPACE_WRITE_PERMISSIONS)? {
            desired.push((path.clone(), PersistentAclKind::Write));
        }
    }
    if desired.is_empty() {
        crate::logging::event(
            "access_check",
            format!("all filesystem access already satisfied for {account}; no ACL mutation"),
        );
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(desired.iter().map(|(path, _)| path.as_path()))?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let sid_bytes = SidBuffer::copy_from_sid(sid)?.0;
    let write_sid_bytes = SidBuffer::copy_from_sid(write_sid)?.0;
    let mut applied_entries = Vec::new();

    for (path, kind) in desired {
        let (entry_account, entry_sid_bytes, acl_sid) = if kind == PersistentAclKind::Write {
            (write_account, &write_sid_bytes, write_sid)
        } else {
            (account, &sid_bytes, sid)
        };
        let entry = PersistentAclEntry {
            account: entry_account.to_string(),
            path: path.clone(),
            kind: kind.clone(),
            sid: entry_sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
        if ledger.entries.contains(&entry) {
            continue;
        }
        crate::logging::event(
            "apply_acl",
            format!(
                "applying persistent {:?} permissions for {account} to {}",
                kind,
                path.display()
            ),
        );
        match kind {
            PersistentAclKind::Read => transaction.grant(
                &path,
                acl_sid,
                true,
                FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            )?,
            PersistentAclKind::Write => {
                transaction.grant(&path, acl_sid, true, WORKSPACE_WRITE_PERMISSIONS)?
            }
            PersistentAclKind::DenyRead => transaction.deny(
                &path,
                acl_sid,
                true,
                FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            )?,
            PersistentAclKind::DenyWrite => unreachable!(),
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    if !applied_entries.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        for entry in applied_entries {
            latest.entries.retain(|existing| {
                existing.account != entry.account
                    || existing.path != entry.path
                    || existing.kind != entry.kind
            });
            latest.entries.push(entry);
        }
        save_acl_ledger(&latest)?;
    }
    transaction.commit();
    Ok(())
}

/// Version 2 expressed protected paths with identity-wide deny ACEs. Those
/// denies cannot represent one approval scope, so remove them before switching
/// to capability-scoped deny ACEs. Dedicated-user write ACEs remain the normal
/// side of WRITE_RESTRICTED's two access checks; the capability SID supplies
/// the independent restricted side and prevents ambient writes from widening
/// the target process policy.
pub(super) fn migrate_legacy_dedicated_user_acls(
    request: &SandboxRequest,
    account: &str,
) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let stale = ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.account.eq_ignore_ascii_case(account)
                && entry.kind == PersistentAclKind::DenyWrite
        })
        .cloned()
        .collect::<Vec<_>>();
    if stale.is_empty() {
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(stale.iter().map(|entry| entry.path.as_path()))?;
    let mut ledger = load_acl_ledger()?;
    let mut revoked = BTreeSet::new();
    for entry in &stale {
        let mut sid = acl_entry_sid(entry)?;
        let key = (entry.path.clone(), sid.0.clone());
        if revoked.insert(key) {
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    ledger.entries.retain(|entry| {
        !entry.account.eq_ignore_ascii_case(account) || entry.kind != PersistentAclKind::DenyWrite
    });
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest.entries.retain(|entry| !stale.contains(entry));
    save_acl_ledger(&latest)?;
    crate::logging::event(
        "migrate_acl",
        format!(
            "removed {} identity-wide protected-path deny ACL entries for {account}; capability_scope={}",
            stale.len(),
            capability_principal(request)
        ),
    );
    Ok(())
}

/// Older versions granted each dedicated account a direct write ACE. Once the
/// account SID is included as a restricted SID for native named-pipe IPC, that
/// direct ACE can satisfy both access checks and reopen another workspace's
/// historical grant. Move the normal-token half of every managed write grant
/// to a stable local group; the per-command capability SID remains the only
/// restricted identity with a write ACE for the current policy scope.
pub(super) fn migrate_dedicated_user_write_acls_to_group(
    account: &str,
    group: &str,
    group_sid: PSID,
) -> Result<()> {
    let preliminary = load_acl_ledger()?;
    let stale_paths = preliminary
        .entries
        .iter()
        .filter(|entry| {
            entry.account.eq_ignore_ascii_case(account) && entry.kind == PersistentAclKind::Write
        })
        .map(|entry| entry.path.clone())
        .collect::<BTreeSet<_>>();
    if stale_paths.is_empty() {
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(stale_paths.iter().map(|path| path.as_path()))?;
    let ledger = load_acl_ledger()?;
    let account_entries = ledger
        .entries
        .iter()
        .filter(|entry| {
            entry.account.eq_ignore_ascii_case(account) && stale_paths.contains(&entry.path)
        })
        .cloned()
        .collect::<Vec<_>>();
    let mut account_sid = account_sid(account)?;
    let group_sid_bytes = SidBuffer::copy_from_sid(group_sid)?.0;
    let mut transaction = AclTransaction::default();
    let mut replacements = Vec::new();

    for path in &stale_paths {
        transaction.grant(path, group_sid, true, WORKSPACE_WRITE_PERMISSIONS)?;
        update_dacl(path, account_sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        replacements.push(PersistentAclEntry {
            account: group.to_string(),
            path: path.clone(),
            kind: PersistentAclKind::Write,
            sid: group_sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        });
    }
    let propagation_roots = stale_paths.iter().filter(|candidate| {
        !stale_paths
            .iter()
            .any(|other| *candidate != other && path_starts_with(candidate, other))
    });
    for root in propagation_roots {
        propagate_inherited_dacl(root)?;
    }
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest
        .entries
        .retain(|entry| !account_entries.contains(entry));
    for entry in replacements {
        latest.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        latest.entries.push(entry);
    }
    save_acl_ledger(&latest)?;
    transaction.commit();
    crate::logging::event(
        "migrate_acl",
        format!(
            "moved {} direct write ACL roots from {account} to {group}",
            stale_paths.len()
        ),
    );
    Ok(())
}

pub(super) fn ensure_persistent_capability_permissions(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<()> {
    let desired_entries = capability_acl_entries(request, principal, sid)?;
    if desired_entries.is_empty() {
        return Ok(());
    }
    if !desired_entries.is_empty() {
        let ledger = load_acl_ledger()?;
        if desired_entries
            .iter()
            .all(|entry| ledger.entries.contains(entry))
        {
            crate::logging::event(
                "access_check",
                format!("capability ACL already provisioned for {principal}"),
            );
            return Ok(());
        }
    }
    let preliminary_ledger = load_acl_ledger()?;
    let legacy_paths = preliminary_ledger
        .entries
        .iter()
        .filter(|entry| entry.account == LEGACY_CAPABILITY_PRINCIPAL)
        .map(|entry| entry.path.as_path())
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(
        desired_entries
            .iter()
            .map(|entry| entry.path.as_path())
            .chain(legacy_paths),
    )?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let mut applied_entries = Vec::new();
    let legacy_entries = ledger
        .entries
        .iter()
        .filter(|entry| entry.account == LEGACY_CAPABILITY_PRINCIPAL)
        .cloned()
        .collect::<Vec<_>>();
    if !legacy_entries.is_empty() {
        let mut legacy_sid = SidBuffer::legacy_opentopia_capability();
        for entry in &legacy_entries {
            update_dacl(&entry.path, legacy_sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
        ledger
            .entries
            .retain(|entry| entry.account != LEGACY_CAPABILITY_PRINCIPAL);
    }
    for entry in desired_entries {
        let path = entry.path.clone();
        let kind = entry.kind.clone();
        revoke_replaced_acl_principals(&ledger, &entry)?;
        if ledger.entries.contains(&entry) {
            continue;
        }
        match kind {
            PersistentAclKind::Write => {
                transaction.grant(&path, sid, true, WORKSPACE_WRITE_PERMISSIONS)?
            }
            PersistentAclKind::DenyWrite => transaction.deny_write(&path, sid, true)?,
            PersistentAclKind::Read | PersistentAclKind::DenyRead => unreachable!(),
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    if !legacy_entries.is_empty() || !applied_entries.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        latest
            .entries
            .retain(|entry| entry.account != LEGACY_CAPABILITY_PRINCIPAL);
        for entry in applied_entries {
            latest.entries.retain(|existing| {
                existing.account != entry.account
                    || existing.path != entry.path
                    || existing.kind != entry.kind
            });
            latest.entries.push(entry);
        }
        save_acl_ledger(&latest)?;
    }
    transaction.commit();
    Ok(())
}

fn capability_acl_entries(
    request: &SandboxRequest,
    principal: &str,
    sid: PSID,
) -> Result<Vec<PersistentAclEntry>> {
    let sid_bytes = SidBuffer::copy_from_sid(sid)?.0;
    let approved = &request.filesystem.allow_protected_write;
    Ok(request
        .filesystem
        .write
        .iter()
        .cloned()
        .map(|path| (path, PersistentAclKind::Write))
        .chain(
            request
                .filesystem
                .deny_write
                .iter()
                .filter(|path| path.exists())
                .filter(|path| {
                    !approved.iter().any(|approved_root| {
                        path_starts_with(path, approved_root)
                            || path_starts_with(approved_root, path)
                    })
                })
                .cloned()
                .map(|path| (path, PersistentAclKind::DenyWrite)),
        )
        .map(|(path, kind)| PersistentAclEntry {
            account: principal.to_string(),
            path,
            kind,
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        })
        .collect())
}

fn revoke_replaced_acl_principals(
    ledger: &PersistentAclLedger,
    desired: &PersistentAclEntry,
) -> Result<()> {
    let mut replaced = BTreeSet::new();
    for entry in ledger.entries.iter().filter(|entry| {
        entry.account == desired.account
            && entry.path == desired.path
            && entry.kind == desired.kind
            && !entry.sid.is_empty()
            && entry.sid != desired.sid
    }) {
        if replaced.insert(entry.sid.clone()) {
            let mut sid = SidBuffer(entry.sid.clone());
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    Ok(())
}

pub(crate) fn has_dedicated_user_permissions(accounts: &[&str]) -> Result<bool> {
    Ok(load_acl_ledger()?.entries.iter().any(|entry| {
        accounts
            .iter()
            .any(|account| entry.account.eq_ignore_ascii_case(account))
    }))
}

pub(crate) fn revoke_dedicated_user_permissions(accounts: &[&str]) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let targets = ledger
        .entries
        .iter()
        .filter(|entry| {
            accounts
                .iter()
                .any(|account| entry.account.eq_ignore_ascii_case(account))
        })
        .cloned()
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(targets.iter().map(|entry| entry.path.as_path()))?;
    let mut revoked = BTreeSet::new();
    for entry in &targets {
        let mut sid = acl_entry_sid(entry)?;
        let key = (entry.path.clone(), sid.0.clone());
        if revoked.insert(key) {
            update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
        }
    }
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest.entries.retain(|entry| {
        !accounts
            .iter()
            .any(|account| entry.account.eq_ignore_ascii_case(account))
    });
    save_acl_ledger(&latest)?;
    Ok(())
}

fn acl_entry_sid(entry: &PersistentAclEntry) -> Result<SidBuffer> {
    if entry.sid.is_empty() {
        acl_principal_sid(&entry.account)
    } else {
        Ok(SidBuffer(entry.sid.clone()))
    }
}

pub(crate) fn cleanup_workspace_acl(args: &[String]) -> Result<i32> {
    let workspace = match args {
        [flag, value] if flag == "--workspace" => {
            let path = std::path::PathBuf::from(value);
            if !path.is_absolute() || !path.exists() {
                anyhow::bail!("cleanup workspace must be an existing absolute path")
            }
            path.canonicalize()
                .context("canonicalize cleanup workspace")?
        }
        _ => anyhow::bail!("usage: opentopia-sandbox cleanup --workspace <absolute-path>"),
    };
    let ledger = load_acl_ledger()?;
    let targets = ledger
        .entries
        .iter()
        .filter(|entry| path_starts_with(&entry.path, &workspace))
        .cloned()
        .collect::<Vec<_>>();
    let _guards = NamedAclMutex::acquire_paths(targets.iter().map(|entry| entry.path.as_path()))?;
    let mut revoked = BTreeSet::new();
    for entry in targets
        .iter()
        .filter(|entry| revoked.insert((entry.account.clone(), entry.path.clone())))
    {
        let mut sid = acl_entry_sid(entry)?;
        update_dacl(&entry.path, sid.as_ptr(), REVOKE_ACCESS, false, 0)?;
    }
    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    latest
        .entries
        .retain(|entry| !path_starts_with(&entry.path, &workspace));
    save_acl_ledger(&latest)?;
    crate::logging::event(
        "cleanup_acl",
        format!(
            "workspace={} revoked={}",
            workspace.display(),
            revoked.len()
        ),
    );
    println!(
        "OpenTopia sandbox ACL cleanup complete: workspace={} entries={}",
        workspace.display(),
        revoked.len()
    );
    Ok(0)
}

pub(super) fn path_starts_with(path: &Path, root: &Path) -> bool {
    let path = path
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let root = root
        .to_string_lossy()
        .replace('/', "\\")
        .trim_end_matches('\\')
        .to_ascii_lowercase();
    let path = path.strip_prefix("\\\\?\\").unwrap_or(&path);
    let root = root.strip_prefix("\\\\?\\").unwrap_or(&root);
    path == root
        || path
            .strip_prefix(root)
            .is_some_and(|suffix| suffix.starts_with('\\'))
}

pub(super) fn account_sid(account: &str) -> Result<SidBuffer> {
    let account = wide(OsStr::new(account));
    let mut sid_len = 0;
    let mut domain_len = 0;
    let mut use_type = 0;
    unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            ptr::null_mut(),
            &mut sid_len,
            ptr::null_mut(),
            &mut domain_len,
            &mut use_type,
        );
    }
    if sid_len == 0 {
        return Err(last_error("stage=prepare_sandbox LookupAccountNameW(size)"));
    }
    let mut sid = vec![0_u8; sid_len as usize];
    let mut domain = vec![0_u16; domain_len as usize];
    let found = unsafe {
        LookupAccountNameW(
            ptr::null(),
            account.as_ptr(),
            sid.as_mut_ptr().cast(),
            &mut sid_len,
            domain.as_mut_ptr(),
            &mut domain_len,
            &mut use_type,
        )
    };
    if found == 0 {
        return Err(last_error("stage=prepare_sandbox LookupAccountNameW"));
    }
    sid.truncate(sid_len as usize);
    Ok(SidBuffer(sid))
}

pub(super) fn acl_principal_sid(principal: &str) -> Result<SidBuffer> {
    if principal == LEGACY_CAPABILITY_PRINCIPAL {
        Ok(SidBuffer::legacy_opentopia_capability())
    } else if let Some(value) = principal
        .strip_prefix(CAPABILITY_PRINCIPAL_PREFIX)
        .or_else(|| principal.strip_prefix(LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX))
    {
        let id = Uuid::parse_str(value).context("parse scoped capability principal")?;
        Ok(SidBuffer::opentopia_capability(id))
    } else {
        account_sid(principal)
    }
}

pub(super) fn capability_principal(request: &SandboxRequest) -> String {
    let mut roots = request
        .filesystem
        .write
        .iter()
        .map(|path| normalized_capability_path(path))
        .collect::<Vec<_>>();
    if roots.is_empty() {
        roots.push(normalized_capability_path(&request.cwd));
    }
    roots.extend(
        request
            .filesystem
            .allow_protected_write
            .iter()
            .map(|path| format!("allow:{}", normalized_capability_path(path))),
    );
    roots.extend(
        request
            .filesystem
            .deny_write
            .iter()
            .map(|path| format!("deny:{}", normalized_capability_path(path))),
    );
    roots.sort_unstable();
    roots.dedup();
    let scope = roots.join("\0");
    let namespace = Uuid::from_u128(CAPABILITY_NAMESPACE);
    let id = Uuid::new_v5(&namespace, scope.as_bytes());
    format!("{CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple())
}

#[cfg(test)]
mod tests {
    use super::{
        acl_principal_sid, capability_principal, path_starts_with, SandboxRequest,
        CAPABILITY_PRINCIPAL_PREFIX, LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX,
    };
    use crate::{BackendMode, NetworkMode};
    use std::path::Path;
    use uuid::Uuid;

    #[test]
    fn path_prefix_matching_respects_component_boundaries_and_extended_paths() {
        assert!(path_starts_with(
            Path::new(r"\\?\C:\workspace\nested"),
            Path::new(r"C:\workspace")
        ));
        assert!(!path_starts_with(
            Path::new(r"C:\workspace-other"),
            Path::new(r"C:\workspace")
        ));
    }

    #[test]
    fn filesystem_capability_principal_is_stable_and_scope_specific() {
        let request = capability_request(&[r"C:\workspace", r"C:\sandbox-home"]);
        let reordered = capability_request(&[r"C:\sandbox-home", r"C:\workspace"]);
        let other = capability_request(&[r"C:\other-workspace", r"C:\sandbox-home"]);
        let principal = capability_principal(&request);
        assert_eq!(principal, capability_principal(&reordered));
        assert_ne!(principal, capability_principal(&other));

        let first = acl_principal_sid(&principal).expect("resolve scoped capability principal");
        let second = acl_principal_sid(&capability_principal(&request))
            .expect("resolve stable capability principal");
        assert_eq!(first.0, second.0);
    }

    #[test]
    fn legacy_scoped_capability_principals_remain_resolvable_for_cleanup() {
        let id = Uuid::new_v4();
        let old = format!("{LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple());
        let new = format!("{CAPABILITY_PRINCIPAL_PREFIX}{}", id.simple());
        assert_eq!(
            acl_principal_sid(&old).expect("legacy SID").0,
            acl_principal_sid(&new).expect("current SID").0
        );
    }

    #[test]
    fn dedicated_write_grants_use_the_managed_group_identity() {
        let source = include_str!("acl_persistence.rs");
        let ensure = source
            .split("pub(super) fn ensure_persistent_user_permissions")
            .nth(1)
            .expect("persistent user permission implementation")
            .split("pub(super) fn migrate_legacy_dedicated_user_acls")
            .next()
            .expect("permission implementation boundary");
        assert!(ensure.contains("if kind == PersistentAclKind::Write"));
        assert!(ensure.contains("(write_account, &write_sid_bytes, write_sid)"));

        let migration = source
            .split("pub(super) fn migrate_dedicated_user_write_acls_to_group")
            .nth(1)
            .expect("dedicated write ACL migration")
            .split("pub(super) fn ensure_persistent_capability_permissions")
            .next()
            .expect("migration boundary");
        assert!(migration.contains("transaction.grant(path, group_sid"));
        assert!(migration.contains("REVOKE_ACCESS"));
        assert!(migration.contains("propagate_inherited_dacl(root)"));
    }

    fn capability_request(write_roots: &[&str]) -> SandboxRequest {
        SandboxRequest {
            interactive: false,
            cwd: Path::new(r"C:\workspace").to_path_buf(),
            filesystem: opentopia_sandbox_protocol::FilesystemCapabilities {
                write: write_roots.iter().map(std::path::PathBuf::from).collect(),
                ..Default::default()
            },
            network: NetworkMode::Internet,
            timeout_ms: Some(1_000),
            termination_timeout_ms: 500,
            max_memory_bytes: None,
            max_cpu_time_ms: None,
            max_output_bytes: None,
            backend: BackendMode::Unelevated,
            command: vec!["cmd.exe".to_string()],
        }
    }
}
