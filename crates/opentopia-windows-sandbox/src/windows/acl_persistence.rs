use super::acl_ledger::{
    load_acl_ledger, save_acl_ledger, PersistentAclEntry, PersistentAclKind, PersistentAclLedger,
};
use super::acl_transaction::{
    dacl_has_explicit_access, propagate_inherited_dacl, update_dacl, AclTransaction, NamedAclMutex,
};
use super::process_launch::{last_error, wide};
use super::security_token::{effective_file_access, SidBuffer};
use super::{
    broker_exchange_root, normalized_capability_path, SandboxRequest,
    ACL_ENTRY_PERMISSIONS_VERSION, CAPABILITY_NAMESPACE, CAPABILITY_PRINCIPAL_PREFIX,
    LEGACY_CAPABILITY_PRINCIPAL, LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX,
    MANAGED_PATH_TRAVERSAL_PERMISSIONS, WORKSPACE_WRITE_PERMISSIONS,
};
use anyhow::{Context, Result};
use opentopia_sandbox_protocol::ReadProvisioning;
use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;
use std::ptr;
use uuid::Uuid;
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Security::Authorization::{DENY_ACCESS, GRANT_ACCESS, REVOKE_ACCESS};
use windows_sys::Win32::Security::{LookupAccountNameW, PSID};
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION,
    FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_EXECUTE, FILE_GENERIC_READ, FILE_READ_ATTRIBUTES,
    FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
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
    let object_generation = acl_object_generation(&path);
    let entry = PersistentAclEntry {
        account: account.to_string(),
        path,
        kind: PersistentAclKind::Write,
        sid: SidBuffer::copy_from_sid(sid)?.0,
        permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
        object_generation,
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

fn acl_object_generation(path: &Path) -> String {
    match windows_file_identity(path) {
        Ok(identity) => identity,
        Err(error) => format!("unavailable:{error:#}"),
    }
}

fn windows_file_identity(path: &Path) -> Result<String> {
    let name = wide(path.as_os_str());
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            FILE_READ_ATTRIBUTES,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(last_error("Get directory identity CreateFileW"));
    }
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    let inspected = unsafe { GetFileInformationByHandle(handle, &mut information) };
    let inspection_error = (inspected == 0).then(std::io::Error::last_os_error);
    unsafe { CloseHandle(handle) };
    if let Some(error) = inspection_error {
        anyhow::bail!("GetFileInformationByHandle failed: {error}");
    }
    Ok(format!(
        "{}:{:08x}{:08x}:{}",
        information.dwVolumeSerialNumber,
        information.nFileIndexHigh,
        information.nFileIndexLow,
        information.dwFileAttributes
    ))
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

pub(super) fn verify_managed_runtime_group_permissions(
    request: &SandboxRequest,
    group: &str,
    token: HANDLE,
) -> Result<()> {
    let ledger = load_acl_ledger()?;
    let mut missing = Vec::new();
    let roots = request
        .filesystem
        .managed_runtime_roots
        .iter()
        .filter(|path| path.is_dir())
        .cloned()
        .collect::<BTreeSet<_>>();
    for root in &roots {
        let recorded = ledger.entries.iter().any(|entry| {
            entry.account.eq_ignore_ascii_case(group)
                && entry.path == *root
                && entry.kind == PersistentAclKind::ManagedRuntimeRead
                && entry.permissions_version == ACL_ENTRY_PERMISSIONS_VERSION
                && entry.object_generation == acl_object_generation(root)
        });
        if !recorded
            || !effective_file_access(token, root, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?
        {
            missing.push(root.display().to_string());
        }
    }
    for ancestor in managed_path_traversal_ancestors(request) {
        if !effective_file_access(token, &ancestor, MANAGED_PATH_TRAVERSAL_PERMISSIONS)? {
            missing.push(format!("traverse:{}", ancestor.display()));
        }
    }
    for capability in request
        .filesystem
        .read_execute
        .iter()
        .filter(|capability| capability.provisioning == ReadProvisioning::Managed)
        .filter(|capability| {
            request
                .filesystem
                .managed_runtime_roots
                .iter()
                .any(|root| path_starts_with(&capability.path, root))
        })
    {
        if !effective_file_access(
            token,
            &capability.path,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )? {
            missing.push(capability.path.display().to_string());
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    missing.sort();
    missing.dedup();
    anyhow::bail!(
        "stage=provision_acl managed runtime group '{group}' is not prepared for this policy scope: {}. Run `opentopia-sandbox provision` before command startup",
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
    let mut missing = Vec::new();
    for entry in &desired {
        if !ledger.entries.contains(entry) || !capability_acl_is_installed(entry, sid)? {
            missing.push(format!("{:?}:{}", entry.kind, entry.path.display()));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "stage=provision_acl capability scope '{principal}' is not prepared: {}. Run `opentopia-sandbox provision` before command startup",
        missing.join(", ")
    )
}

fn managed_path_traversal_ancestors(request: &SandboxRequest) -> BTreeSet<std::path::PathBuf> {
    let managed_paths = request
        .filesystem
        .read_execute
        .iter()
        .filter(|capability| capability.provisioning == ReadProvisioning::Managed)
        .map(|capability| capability.path.as_path())
        .chain(request.filesystem.write.iter().map(|path| path.as_path()))
        .chain(
            request
                .filesystem
                .managed_runtime_roots
                .iter()
                .map(|path| path.as_path()),
        );
    let mut ancestors = BTreeSet::new();
    for path in managed_paths.filter(|path| path.exists()) {
        for ancestor in path.ancestors().skip(1) {
            if ancestor.is_dir() {
                ancestors.insert(ancestor.to_path_buf());
            }
        }
    }
    ancestors
}

/// Reconcile OpenTopia-managed roots against the stable sandbox group.
/// Runtime parents receive inheritable RX for future generations; ancestors
/// of any managed read/write root receive only a non-inheriting
/// traverse/read-attributes ACE when their effective access is missing.
pub(super) fn ensure_managed_runtime_group_permissions(
    request: &SandboxRequest,
    group: &str,
    group_sid: PSID,
    token: HANDLE,
) -> Result<()> {
    let roots = request
        .filesystem
        .managed_runtime_roots
        .iter()
        .filter(|path| path.is_dir())
        .cloned()
        .collect::<BTreeSet<_>>();
    let targets = request
        .filesystem
        .read_execute
        .iter()
        .filter(|capability| capability.provisioning == ReadProvisioning::Managed)
        .map(|capability| capability.path.clone())
        .filter(|path| roots.iter().any(|root| path_starts_with(path, root)))
        .collect::<BTreeSet<_>>();
    let ancestors = managed_path_traversal_ancestors(request);
    let sid_bytes = SidBuffer::copy_from_sid(group_sid)?.0;
    let ledger = load_acl_ledger()?;
    let root_is_recorded = |root: &Path| {
        ledger.entries.iter().any(|entry| {
            entry.account.eq_ignore_ascii_case(group)
                && entry.path == root
                && entry.kind == PersistentAclKind::ManagedRuntimeRead
                && entry.sid == sid_bytes
                && entry.permissions_version == ACL_ENTRY_PERMISSIONS_VERSION
                && entry.object_generation == acl_object_generation(root)
        })
    };
    let historical_root_is_recorded = |root: &Path| {
        ledger.entries.iter().any(|entry| {
            entry.account.eq_ignore_ascii_case(group)
                && entry.path == root
                && entry.kind == PersistentAclKind::ManagedRuntimeRead
                && entry.sid == sid_bytes
                && entry.permissions_version == ACL_ENTRY_PERMISSIONS_VERSION
        })
    };
    let mut roots_to_reconcile = BTreeSet::new();
    let mut root_ledger_updates = Vec::new();
    for root in &roots {
        let effective =
            effective_file_access(token, root, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?;
        if !root_is_recorded(root) && historical_root_is_recorded(root) && effective {
            root_ledger_updates.push(PersistentAclEntry {
                account: group.to_string(),
                path: root.clone(),
                kind: PersistentAclKind::ManagedRuntimeRead,
                sid: sid_bytes.clone(),
                permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
                object_generation: acl_object_generation(root),
            });
        } else if !root_is_recorded(root) || !effective {
            roots_to_reconcile.insert(root.clone());
        }
    }
    for target in &targets {
        if !effective_file_access(token, target, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)? {
            roots_to_reconcile.extend(
                roots
                    .iter()
                    .filter(|root| path_starts_with(target, root))
                    .cloned(),
            );
        }
    }
    let mut inaccessible_ancestors = Vec::new();
    for ancestor in &ancestors {
        if !effective_file_access(token, ancestor, MANAGED_PATH_TRAVERSAL_PERMISSIONS)? {
            inaccessible_ancestors.push(ancestor.clone());
        }
    }
    if !root_ledger_updates.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        for entry in root_ledger_updates {
            latest.entries.retain(|existing| {
                !existing.account.eq_ignore_ascii_case(&entry.account)
                    || existing.path != entry.path
                    || existing.kind != entry.kind
            });
            latest.entries.push(entry);
        }
        save_acl_ledger(&latest)?;
    }
    if roots_to_reconcile.is_empty() && inaccessible_ancestors.is_empty() {
        crate::logging::event(
            "access_check",
            format!("managed runtime group ACL already effective for {group}"),
        );
        return Ok(());
    }

    let _guards = NamedAclMutex::acquire_paths(
        roots_to_reconcile
            .iter()
            .map(|path| path.as_path())
            .chain(targets.iter().map(|path| path.as_path()))
            .chain(inaccessible_ancestors.iter().map(|path| path.as_path())),
    )?;
    let mut ledger = load_acl_ledger()?;
    let mut transaction = AclTransaction::default();
    let mut applied_entries = Vec::new();
    // Native runtimes such as Node query metadata for every path component.
    // Grant only traverse/read-attributes on inaccessible ancestors and do not
    // inherit it, so the sandbox still cannot list or read sibling content.
    for ancestor in &inaccessible_ancestors {
        let entry = PersistentAclEntry {
            account: group.to_string(),
            path: ancestor.clone(),
            kind: PersistentAclKind::ManagedRuntimeTraverse,
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
            object_generation: acl_object_generation(ancestor),
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
        transaction.grant_without_child_propagation(
            ancestor,
            group_sid,
            MANAGED_PATH_TRAVERSAL_PERMISSIONS,
        )?;
        ledger.entries.retain(|existing| {
            !existing.account.eq_ignore_ascii_case(&entry.account)
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    for root in &roots_to_reconcile {
        let entry = PersistentAclEntry {
            account: group.to_string(),
            path: root.clone(),
            kind: PersistentAclKind::ManagedRuntimeRead,
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
            object_generation: acl_object_generation(root),
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
        crate::logging::event(
            "apply_acl",
            format!(
                "reconciling inherited managed runtime permissions for {group} at {}",
                root.display()
            ),
        );
        transaction.grant(
            root,
            group_sid,
            true,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )?;
        propagate_inherited_dacl(root)?;
        ledger.entries.retain(|existing| {
            !existing.account.eq_ignore_ascii_case(&entry.account)
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    // A child can explicitly disable inheritance. Repair only those managed
    // generation roots that remain inaccessible after parent propagation.
    for target in &targets {
        if effective_file_access(token, target, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)? {
            continue;
        }
        let entry = PersistentAclEntry {
            account: group.to_string(),
            path: target.clone(),
            kind: PersistentAclKind::ManagedRuntimeRead,
            sid: sid_bytes.clone(),
            permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
            object_generation: acl_object_generation(target),
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
        transaction.grant(
            target,
            group_sid,
            true,
            FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
        )?;
        ledger.entries.retain(|existing| {
            !existing.account.eq_ignore_ascii_case(&entry.account)
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        applied_entries.push(entry);
    }
    for target in roots.iter().chain(targets.iter()) {
        anyhow::ensure!(
            effective_file_access(token, target, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)?,
            "stage=apply_acl managed runtime group {group} is still unable to read/execute {} after ACL reconciliation",
            target.display()
        );
    }
    for ancestor in &ancestors {
        anyhow::ensure!(
            effective_file_access(token, ancestor, MANAGED_PATH_TRAVERSAL_PERMISSIONS)?,
            "stage=apply_acl managed runtime group {group} is still unable to traverse {} after ACL reconciliation",
            ancestor.display()
        );
    }

    let _ledger_guard = NamedAclMutex::acquire_metadata()?;
    let mut latest = load_acl_ledger()?;
    for entry in applied_entries {
        latest.entries.retain(|existing| {
            !existing.account.eq_ignore_ascii_case(&entry.account)
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        latest.entries.push(entry);
    }
    save_acl_ledger(&latest)?;
    transaction.commit();
    Ok(())
}

pub(super) fn ensure_persistent_user_permissions(
    request: &SandboxRequest,
    account: &str,
    sid: PSID,
    write_account: &str,
    write_sid: PSID,
    token: HANDLE,
) -> Result<()> {
    let mut desired = Vec::new();
    for path in request
        .filesystem
        .deny_read
        .iter()
        .filter(|path| path.exists())
    {
        if effective_file_access(token, path, FILE_GENERIC_READ | FILE_GENERIC_EXECUTE)? {
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
            object_generation: acl_object_generation(&path),
        };
        revoke_replaced_acl_principals(&ledger, &entry)?;
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
            PersistentAclKind::ManagedRuntimeRead | PersistentAclKind::ManagedRuntimeTraverse => {
                unreachable!()
            }
            PersistentAclKind::Write => {
                transaction.grant(&path, acl_sid, true, WORKSPACE_WRITE_PERMISSIONS)?
            }
            PersistentAclKind::DenyRead => transaction.deny(
                &path,
                acl_sid,
                true,
                FILE_GENERIC_READ | FILE_GENERIC_EXECUTE,
            )?,
            PersistentAclKind::DenyWrite | PersistentAclKind::Unknown(_) => unreachable!(),
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
            object_generation: acl_object_generation(path),
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
    let existing_ledger = load_acl_ledger()?;
    if desired_entries
        .iter()
        .all(|entry| existing_ledger.entries.contains(entry))
    {
        crate::logging::event(
            "access_check",
            format!("capability ACL already provisioned for {principal}"),
        );
        return Ok(());
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
    let mut ledger_updates = Vec::new();
    for desired in &desired_entries {
        if ledger.entries.contains(desired) {
            continue;
        }
        let historical = ledger.entries.iter().any(|entry| {
            entry.account == desired.account
                && entry.path == desired.path
                && entry.kind == desired.kind
                && entry.sid == desired.sid
                && entry.permissions_version == desired.permissions_version
        });
        if historical && capability_acl_is_installed(desired, sid)? {
            ledger.entries.retain(|entry| {
                entry.account != desired.account
                    || entry.path != desired.path
                    || entry.kind != desired.kind
            });
            ledger.entries.push(desired.clone());
            ledger_updates.push(desired.clone());
        }
    }
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
            PersistentAclKind::Read
            | PersistentAclKind::ManagedRuntimeRead
            | PersistentAclKind::ManagedRuntimeTraverse
            | PersistentAclKind::DenyRead
            | PersistentAclKind::Unknown(_) => unreachable!(),
        }
        ledger.entries.retain(|existing| {
            existing.account != entry.account
                || existing.path != entry.path
                || existing.kind != entry.kind
        });
        ledger.entries.push(entry.clone());
        ledger_updates.push(entry);
    }
    if !legacy_entries.is_empty() || !ledger_updates.is_empty() {
        let _ledger_guard = NamedAclMutex::acquire_metadata()?;
        let mut latest = load_acl_ledger()?;
        latest
            .entries
            .retain(|entry| entry.account != LEGACY_CAPABILITY_PRINCIPAL);
        for entry in ledger_updates {
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

fn capability_acl_is_installed(entry: &PersistentAclEntry, sid: PSID) -> Result<bool> {
    match entry.kind {
        PersistentAclKind::Write => {
            dacl_has_explicit_access(&entry.path, sid, GRANT_ACCESS, WORKSPACE_WRITE_PERMISSIONS)
        }
        PersistentAclKind::DenyWrite => dacl_has_explicit_access(
            &entry.path,
            sid,
            DENY_ACCESS,
            super::WRITE_RESTRICTION_PERMISSIONS,
        ),
        PersistentAclKind::Read
        | PersistentAclKind::ManagedRuntimeRead
        | PersistentAclKind::ManagedRuntimeTraverse
        | PersistentAclKind::DenyRead
        | PersistentAclKind::Unknown(_) => Ok(false),
    }
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
        .map(|(path, kind)| {
            let object_generation = acl_object_generation(&path);
            PersistentAclEntry {
                account: principal.to_string(),
                path,
                kind,
                sid: sid_bytes.clone(),
                permissions_version: ACL_ENTRY_PERMISSIONS_VERSION,
                object_generation,
            }
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
    reject_unknown_cleanup_entries(&targets, "dedicated-user sandbox teardown")?;
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
    reject_unknown_cleanup_entries(&targets, "workspace ACL cleanup")?;
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

fn reject_unknown_cleanup_entries(entries: &[PersistentAclEntry], operation: &str) -> Result<()> {
    if let Some(entry) = entries.iter().find(|entry| entry.kind.is_unknown()) {
        anyhow::bail!(
            "{operation} requires a newer sandbox helper because ACL kind {:?} at {} is unknown; the entry was preserved without changing its ACL",
            entry.kind,
            entry.path.display()
        )
    }
    Ok(())
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
        acl_object_generation, acl_principal_sid, capability_principal,
        managed_path_traversal_ancestors, path_starts_with, SandboxRequest,
        CAPABILITY_PRINCIPAL_PREFIX, LEGACY_SCOPED_CAPABILITY_PRINCIPAL_PREFIX,
    };
    use crate::{BackendMode, NetworkMode};
    use opentopia_sandbox_protocol::{ReadExecuteCapability, ReadProvisioning};
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

    #[test]
    fn acl_object_generation_changes_when_a_directory_is_replaced() {
        let path =
            std::env::temp_dir().join(format!("opentopia-acl-generation-test-{}", Uuid::new_v4()));
        std::fs::create_dir(&path).expect("create first directory generation");
        let first = acl_object_generation(&path);
        std::fs::remove_dir(&path).expect("remove first directory generation");
        std::fs::create_dir(&path).expect("create replacement directory generation");
        let replacement = acl_object_generation(&path);
        std::fs::remove_dir(&path).expect("remove replacement directory generation");

        assert_ne!(first, replacement);
    }

    #[test]
    fn traversal_ancestors_cover_managed_paths_but_not_external_runtime_branches() {
        let root = std::env::temp_dir().join(format!(
            "opentopia-managed-traversal-test-{}",
            Uuid::new_v4()
        ));
        let managed_branch = root.join("managed-branch");
        let managed_leaf = managed_branch.join("leaf");
        let write_branch = root.join("write-branch");
        let write_leaf = write_branch.join("leaf");
        let external_branch = root.join("external-branch");
        let external_leaf = external_branch.join("leaf");
        let stable_runtime = root.join("runtime-parent");
        for path in [&managed_leaf, &write_leaf, &external_leaf, &stable_runtime] {
            std::fs::create_dir_all(path).expect("create traversal test path");
        }
        let mut request = capability_request(&[]);
        request.filesystem.read_execute = vec![
            ReadExecuteCapability {
                path: managed_leaf,
                provisioning: ReadProvisioning::Managed,
            },
            ReadExecuteCapability {
                path: external_leaf,
                provisioning: ReadProvisioning::ExistingOnly,
            },
        ];
        request.filesystem.write = vec![write_leaf];
        request.filesystem.managed_runtime_roots = vec![stable_runtime];

        let ancestors = managed_path_traversal_ancestors(&request);
        assert!(ancestors.contains(&managed_branch));
        assert!(ancestors.contains(&write_branch));
        assert!(!ancestors.contains(&external_branch));

        std::fs::remove_dir_all(&root).expect("remove traversal test tree");
    }

    fn capability_request(write_roots: &[&str]) -> SandboxRequest {
        SandboxRequest {
            interactive: false,
            persistent_stdio: false,
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
