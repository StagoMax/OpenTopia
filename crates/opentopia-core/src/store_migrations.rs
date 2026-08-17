//! Verified SQLite schema migration boundary.
//!
//! Versions through v19 belong to the quarantined legacy importer in
//! `store.rs`. A legacy database receives a v19 baseline only after its actual
//! semantic schema matches a freshly built canonical v19 schema. Starting at
//! v20, migrations are immutable SQL resources with ordered registry entries,
//! SHA-256 checksums, transactional application, post-migration verification,
//! and a schema fingerprint. Never edit a released migration resource; add the
//! next contiguous version instead.

use anyhow::Context;
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fmt::Write as _;

pub(crate) const LEGACY_DATABASE_SCHEMA_VERSION: i64 = 19;
pub(crate) const CURRENT_DATABASE_SCHEMA_VERSION: i64 = 21;

const LEGACY_BASELINE_NAME: &str = "legacy_baseline_v19";
const MIGRATION_LEDGER_SQL: &str = include_str!("migrations/0019_legacy_baseline.sql");
const LEGACY_CANONICAL_MANIFEST_SHA256: &str =
    "sha256:30e0285a7919252a39f23c19ace6bf8166f8498b377474718c1e9e75ef5bccc9";

// These tables belonged to development-only goal/evaluation experiments that
// predate the verified migration boundary. They are intentionally retained as
// opaque legacy data: current runtime code neither reads nor writes them, while
// excluding them from the canonical manifest lets an existing development
// database cross the v19 baseline without deleting historical evaluation data.
// The full schema fingerprint still locks their exact post-baseline shape.
const PRESERVED_LEGACY_TABLES: &[&str] = &[
    "evaluation_runs",
    "goal_plan_revisions",
    "goal_task_attempts",
    "goal_tasks",
    "harness_candidate_evaluations",
    "harness_candidates",
    "harness_experience_rule_evaluation_threads",
    "harness_experience_rule_evaluations",
    "harness_experience_rules",
    "harness_learning_cases",
    "harness_promotion_decisions",
    "harness_releases",
    "harness_review_candidates",
    "harness_user_corrections",
    "harness_workflow_reviews",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SchemaManifest {
    entries: Vec<String>,
}

#[derive(Debug, Clone)]
struct MigrationRecord {
    version: i64,
    name: String,
    checksum: String,
    schema_fingerprint: String,
}

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
    verify: fn(&Connection) -> anyhow::Result<()>,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 20,
        name: "verified_migration_ledger",
        sql: include_str!("migrations/0020_verified_migration_ledger.sql"),
        verify: verify_v20,
    },
    Migration {
        version: 21,
        name: "agent_collaboration_domain",
        sql: include_str!("migrations/0021_agent_collaboration_domain.sql"),
        verify: verify_v21,
    },
];

pub(crate) fn has_migration_ledger(conn: &Connection) -> anyhow::Result<bool> {
    table_exists(conn, "schema_migrations")
}

pub(crate) fn preflight_unmanaged_database(conn: &Connection) -> anyhow::Result<()> {
    let user_version = user_version(conn)?;
    anyhow::ensure!(
        user_version <= CURRENT_DATABASE_SCHEMA_VERSION,
        "database schema version {user_version} is newer than supported version {CURRENT_DATABASE_SCHEMA_VERSION}"
    );
    anyhow::ensure!(
        user_version <= LEGACY_DATABASE_SCHEMA_VERSION,
        "database schema version {user_version} requires a migration ledger, but schema_migrations is missing"
    );
    Ok(())
}

pub(crate) fn validate_managed_database_before_migration(conn: &Connection) -> anyhow::Result<()> {
    let user_version = user_version(conn)?;
    anyhow::ensure!(
        user_version <= CURRENT_DATABASE_SCHEMA_VERSION,
        "database schema version {user_version} is newer than supported version {CURRENT_DATABASE_SCHEMA_VERSION}"
    );
    validate_ledger_shape(conn)?;
    let records = load_migration_records(conn)?;
    validate_migration_history(&records, user_version)?;
    let latest = records
        .last()
        .context("schema_migrations must contain a legacy baseline")?;
    let actual_fingerprint = schema_fingerprint(conn)?;
    anyhow::ensure!(
        latest.schema_fingerprint == actual_fingerprint,
        "database schema drift detected at version {}: expected fingerprint {}, found {}",
        latest.version,
        latest.schema_fingerprint,
        actual_fingerprint
    );
    Ok(())
}

pub(crate) fn initialize_legacy_baseline(conn: &mut Connection) -> anyhow::Result<()> {
    anyhow::ensure!(
        !has_migration_ledger(conn)?,
        "cannot initialize a legacy baseline over an existing schema_migrations table"
    );
    anyhow::ensure!(
        user_version(conn)? == LEGACY_DATABASE_SCHEMA_VERSION,
        "legacy schema must be reconciled to version {LEGACY_DATABASE_SCHEMA_VERSION} before baselining"
    );
    let checksum = definition_checksum(
        LEGACY_DATABASE_SCHEMA_VERSION,
        LEGACY_BASELINE_NAME,
        MIGRATION_LEDGER_SQL,
    );
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    tx.execute_batch(MIGRATION_LEDGER_SQL)?;
    let fingerprint = schema_fingerprint(&tx)?;
    insert_migration_record(
        &tx,
        LEGACY_DATABASE_SCHEMA_VERSION,
        LEGACY_BASELINE_NAME,
        &checksum,
        &fingerprint,
    )?;
    tx.commit()?;
    Ok(())
}

pub(crate) fn apply_pending_migrations(conn: &mut Connection) -> anyhow::Result<()> {
    validate_managed_database_before_migration(conn)?;
    let mut latest = load_migration_records(conn)?
        .last()
        .map(|record| record.version)
        .context("schema_migrations must contain a legacy baseline")?;

    if MIGRATIONS
        .iter()
        .all(|migration| migration.version <= latest)
    {
        return Ok(());
    }
    validate_database_integrity(conn)?;

    for migration in MIGRATIONS {
        if migration.version <= latest {
            continue;
        }
        anyhow::ensure!(
            migration.version == latest + 1,
            "migration registry has a gap after version {latest}: next version is {}",
            migration.version
        );
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute_batch(migration.sql).with_context(|| {
            format!(
                "failed to apply migration v{} {}",
                migration.version, migration.name
            )
        })?;
        (migration.verify)(&tx).with_context(|| {
            format!(
                "failed to verify migration v{} {}",
                migration.version, migration.name
            )
        })?;
        validate_database_integrity(&tx)?;
        let fingerprint = schema_fingerprint(&tx)?;
        let checksum = migration_checksum(migration);
        insert_migration_record(
            &tx,
            migration.version,
            migration.name,
            &checksum,
            &fingerprint,
        )?;
        tx.pragma_update(None, "user_version", migration.version)?;
        tx.commit()?;
        latest = migration.version;
    }
    Ok(())
}

pub(crate) fn verify_current_database(
    conn: &Connection,
    expected_schema: &SchemaManifest,
) -> anyhow::Result<()> {
    validate_managed_database_before_migration(conn)?;
    let records = load_migration_records(conn)?;
    let latest = records
        .last()
        .context("schema_migrations must contain a migration record")?;
    anyhow::ensure!(
        latest.version == CURRENT_DATABASE_SCHEMA_VERSION,
        "database migration stopped at version {}; current version is {}",
        latest.version,
        CURRENT_DATABASE_SCHEMA_VERSION
    );
    validate_schema_manifest(conn, expected_schema)
}

pub(crate) fn inspect_schema(conn: &Connection) -> anyhow::Result<SchemaManifest> {
    let mut tables = Vec::new();
    {
        let mut stmt = conn.prepare(
            r#"
            SELECT name
            FROM sqlite_schema
            WHERE type = 'table'
              AND name NOT LIKE 'sqlite_%'
              AND name <> 'schema_migrations'
            ORDER BY name
            "#,
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        for row in rows {
            let table = row?;
            if !is_preserved_legacy_table(&table) {
                tables.push(table);
            }
        }
    }

    let mut entries = Vec::new();
    for table in tables {
        entries.push(format!("table:{table}"));
        let quoted_table = quote_identifier(&table);

        let mut columns = Vec::new();
        {
            let mut stmt = conn.prepare(&format!("PRAGMA table_xinfo({quoted_table})"))?;
            let rows = stmt.query_map([], |row| {
                let name: String = row.get(1)?;
                let data_type: String = row.get(2)?;
                let not_null: i64 = row.get(3)?;
                let default_value: Option<String> = row.get(4)?;
                let primary_key: i64 = row.get(5)?;
                let hidden: i64 = row.get(6)?;
                Ok(format!(
                    "column:{table}:{name}:{}:{not_null}:{}:{primary_key}:{hidden}",
                    data_type.to_ascii_uppercase(),
                    default_value
                        .as_deref()
                        .map(normalize_sql)
                        .unwrap_or_else(|| "<null>".to_string())
                ))
            })?;
            for row in rows {
                columns.push(row?);
            }
        }
        columns.sort();
        entries.extend(columns);

        let mut foreign_keys = Vec::new();
        {
            let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list({quoted_table})"))?;
            let rows = stmt.query_map([], |row| {
                let referenced_table: String = row.get(2)?;
                let from: String = row.get(3)?;
                let to: Option<String> = row.get(4)?;
                let on_update: String = row.get(5)?;
                let on_delete: String = row.get(6)?;
                let match_kind: String = row.get(7)?;
                Ok(format!(
                    "foreign-key:{table}:{from}:{referenced_table}:{}:{}:{}:{}",
                    to.unwrap_or_default(),
                    on_update.to_ascii_uppercase(),
                    on_delete.to_ascii_uppercase(),
                    match_kind.to_ascii_uppercase()
                ))
            })?;
            for row in rows {
                foreign_keys.push(row?);
            }
        }
        foreign_keys.sort();
        entries.extend(foreign_keys);

        if let Some(create_sql) = table_sql(conn, &table)? {
            let mut checks = extract_check_constraints(&create_sql);
            checks.sort();
            entries.extend(
                checks
                    .into_iter()
                    .map(|check| format!("check:{table}:{check}")),
            );
        }

        let mut indexes = Vec::new();
        let mut index_rows = Vec::new();
        {
            let mut stmt = conn.prepare(&format!("PRAGMA index_list({quoted_table})"))?;
            let rows = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            for row in rows {
                index_rows.push(row?);
            }
        }
        for (index_name, unique, origin, partial) in index_rows {
            let quoted_index = quote_identifier(&index_name);
            let mut index_columns = Vec::new();
            let mut stmt = conn.prepare(&format!("PRAGMA index_xinfo({quoted_index})"))?;
            let rows = stmt.query_map([], |row| {
                let sequence: i64 = row.get(0)?;
                let name: Option<String> = row.get(2)?;
                let descending: i64 = row.get(3)?;
                let collation: Option<String> = row.get(4)?;
                let key: i64 = row.get(5)?;
                Ok((
                    sequence,
                    format!(
                        "{sequence}:{}:{descending}:{}:{key}",
                        name.unwrap_or_else(|| "<expression>".to_string()),
                        collation.unwrap_or_default().to_ascii_uppercase()
                    ),
                ))
            })?;
            for row in rows {
                index_columns.push(row?);
            }
            index_columns.sort_by_key(|(sequence, _)| *sequence);
            let predicate = index_sql(conn, &index_name)?
                .and_then(|sql| sql_where_clause(&sql))
                .unwrap_or_default();
            indexes.push(format!(
                "index:{table}:{index_name}:{unique}:{}:{partial}:{predicate}:{}",
                origin.to_ascii_lowercase(),
                index_columns
                    .into_iter()
                    .map(|(_, column)| column)
                    .collect::<Vec<_>>()
                    .join("|")
            ));
        }
        indexes.sort();
        entries.extend(indexes);
    }

    let mut programmable_objects = Vec::new();
    {
        let mut stmt = conn.prepare(
            r#"
            SELECT type, name, tbl_name, sql
            FROM sqlite_schema
            WHERE type IN ('view', 'trigger') AND sql IS NOT NULL
            ORDER BY type, name
            "#,
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
            ))
        })?;
        for row in rows {
            let (object_type, name, table, sql) = row?;
            if is_preserved_legacy_table(&name) || is_preserved_legacy_table(&table) {
                continue;
            }
            programmable_objects.push(format!(
                "object:{object_type}:{name}:{}",
                normalize_sql(&sql)
            ));
        }
    }
    entries.extend(programmable_objects);
    entries.sort();
    Ok(SchemaManifest { entries })
}

pub(crate) fn validate_schema_manifest(
    conn: &Connection,
    expected: &SchemaManifest,
) -> anyhow::Result<()> {
    let actual = inspect_schema(conn)?;
    if actual == *expected {
        return Ok(());
    }

    let expected_entries = expected.entries.iter().cloned().collect::<BTreeSet<_>>();
    let actual_entries = actual.entries.iter().cloned().collect::<BTreeSet<_>>();
    let missing = expected_entries
        .difference(&actual_entries)
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    let unexpected = actual_entries
        .difference(&expected_entries)
        .take(8)
        .cloned()
        .collect::<Vec<_>>();
    anyhow::bail!(
        "database schema does not match the canonical manifest; missing [{}]; unexpected [{}]",
        missing.join(", "),
        unexpected.join(", ")
    )
}

pub(crate) fn validate_frozen_legacy_manifest(manifest: &SchemaManifest) -> anyhow::Result<()> {
    let actual = manifest_checksum(manifest);
    anyhow::ensure!(
        actual == LEGACY_CANONICAL_MANIFEST_SHA256,
        "the frozen v{LEGACY_DATABASE_SCHEMA_VERSION} canonical manifest changed: expected {LEGACY_CANONICAL_MANIFEST_SHA256}, found {actual}; add a new migration instead of editing the legacy baseline"
    );
    Ok(())
}

pub(crate) fn validate_database_integrity(conn: &Connection) -> anyhow::Result<()> {
    let quick_check: String = conn.query_row("PRAGMA quick_check(1)", [], |row| row.get(0))?;
    anyhow::ensure!(
        quick_check.eq_ignore_ascii_case("ok"),
        "SQLite quick_check failed: {quick_check}"
    );

    let mut stmt = conn.prepare("PRAGMA foreign_key_check")?;
    let violation = stmt
        .query_row([], |row| {
            Ok(format!(
                "table={}, rowid={}, parent={}, constraint={}",
                row.get::<_, String>(0)?,
                row.get::<_, Option<i64>>(1)?
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "<none>".to_string()),
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?
            ))
        })
        .optional()?;
    anyhow::ensure!(
        violation.is_none(),
        "SQLite foreign_key_check failed: {}",
        violation.unwrap_or_default()
    );
    Ok(())
}

fn verify_v20(conn: &Connection) -> anyhow::Result<()> {
    validate_ledger_shape(conn)
}

fn verify_v21(conn: &Connection) -> anyhow::Result<()> {
    for table in [
        "agent_sessions",
        "agent_runtime_snapshots",
        "agent_threads",
        "agent_turns",
        "agent_ledger_items",
        "agent_mailbox_messages",
    ] {
        anyhow::ensure!(table_exists(conn, table)?, "{table} table is missing");
    }
    Ok(())
}

fn validate_ledger_shape(conn: &Connection) -> anyhow::Result<()> {
    anyhow::ensure!(
        table_exists(conn, "schema_migrations")?,
        "schema_migrations table is missing"
    );
    let mut columns = Vec::new();
    let mut stmt = conn.prepare("PRAGMA table_info(schema_migrations)")?;
    let rows = stmt.query_map([], |row| row.get::<_, String>(1))?;
    for row in rows {
        columns.push(row?);
    }
    anyhow::ensure!(
        columns
            == [
                "version",
                "name",
                "checksum",
                "schema_fingerprint",
                "applied_at",
                "app_build",
            ],
        "schema_migrations has an unsupported shape: {}",
        columns.join(", ")
    );
    Ok(())
}

fn load_migration_records(conn: &Connection) -> anyhow::Result<Vec<MigrationRecord>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT version, name, checksum, schema_fingerprint
        FROM schema_migrations
        ORDER BY version
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(MigrationRecord {
            version: row.get(0)?,
            name: row.get(1)?,
            checksum: row.get(2)?,
            schema_fingerprint: row.get(3)?,
        })
    })?;
    let mut records = Vec::new();
    for row in rows {
        records.push(row?);
    }
    Ok(records)
}

fn validate_migration_history(
    records: &[MigrationRecord],
    user_version: i64,
) -> anyhow::Result<()> {
    let baseline = records.first().context("schema_migrations is empty")?;
    anyhow::ensure!(
        baseline.version == LEGACY_DATABASE_SCHEMA_VERSION
            && baseline.name == LEGACY_BASELINE_NAME,
        "schema_migrations must begin with the verified v{LEGACY_DATABASE_SCHEMA_VERSION} legacy baseline"
    );
    let baseline_checksum = definition_checksum(
        LEGACY_DATABASE_SCHEMA_VERSION,
        LEGACY_BASELINE_NAME,
        MIGRATION_LEDGER_SQL,
    );
    anyhow::ensure!(
        baseline.checksum == baseline_checksum,
        "migration checksum mismatch for v{} {}",
        baseline.version,
        baseline.name
    );

    let mut previous_version = baseline.version;
    for record in records.iter().skip(1) {
        anyhow::ensure!(
            record.version == previous_version + 1,
            "schema_migrations has a version gap between {previous_version} and {}",
            record.version
        );
        let migration = MIGRATIONS
            .iter()
            .find(|migration| migration.version == record.version)
            .with_context(|| {
                format!(
                    "database contains unknown migration version {}",
                    record.version
                )
            })?;
        anyhow::ensure!(
            record.name == migration.name,
            "migration name mismatch for v{}: expected {}, found {}",
            record.version,
            migration.name,
            record.name
        );
        anyhow::ensure!(
            record.checksum == migration_checksum(migration),
            "migration checksum mismatch for v{} {}",
            record.version,
            record.name
        );
        previous_version = record.version;
    }

    anyhow::ensure!(
        previous_version <= CURRENT_DATABASE_SCHEMA_VERSION,
        "database migration version {previous_version} is newer than supported version {CURRENT_DATABASE_SCHEMA_VERSION}"
    );
    anyhow::ensure!(
        user_version == previous_version,
        "PRAGMA user_version ({user_version}) does not match the migration ledger ({previous_version})"
    );
    Ok(())
}

fn insert_migration_record(
    conn: &Connection,
    version: i64,
    name: &str,
    checksum: &str,
    schema_fingerprint: &str,
) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO schema_migrations (
            version, name, checksum, schema_fingerprint, applied_at, app_build
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
        "#,
        params![
            version,
            name,
            checksum,
            schema_fingerprint,
            Utc::now().to_rfc3339(),
            env!("CARGO_PKG_VERSION"),
        ],
    )?;
    Ok(())
}

fn migration_checksum(migration: &Migration) -> String {
    definition_checksum(migration.version, migration.name, migration.sql)
}

fn definition_checksum(version: i64, name: &str, definition: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(version.to_le_bytes());
    hasher.update([0]);
    hasher.update(name.as_bytes());
    hasher.update([0]);
    hasher.update(definition.as_bytes());
    finish_sha256(hasher)
}

fn manifest_checksum(manifest: &SchemaManifest) -> String {
    let mut hasher = Sha256::new();
    for entry in &manifest.entries {
        hasher.update(entry.as_bytes());
        hasher.update([0]);
    }
    finish_sha256(hasher)
}

fn schema_fingerprint(conn: &Connection) -> anyhow::Result<String> {
    let mut stmt = conn.prepare(
        r#"
        SELECT type, name, tbl_name, sql
        FROM sqlite_schema
        WHERE name NOT LIKE 'sqlite_%'
          AND sql IS NOT NULL
        ORDER BY type, name
        "#,
    )?;
    let rows = stmt.query_map([], |row| {
        Ok(format!(
            "{}\0{}\0{}\0{}",
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            normalize_sql(&row.get::<_, String>(3)?)
        ))
    })?;
    let mut serialized = String::new();
    for row in rows {
        serialized.push_str(&row?);
        serialized.push('\n');
    }
    let mut hasher = Sha256::new();
    hasher.update(serialized.as_bytes());
    Ok(finish_sha256(hasher))
}

fn finish_sha256(hasher: Sha256) -> String {
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity("sha256:".len() + digest.len() * 2);
    encoded.push_str("sha256:");
    for byte in digest {
        write!(&mut encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

fn user_version(conn: &Connection) -> anyhow::Result<i64> {
    Ok(conn.query_row("PRAGMA user_version", [], |row| row.get(0))?)
}

fn table_exists(conn: &Connection, table: &str) -> anyhow::Result<bool> {
    Ok(conn
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            params![table],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

fn is_preserved_legacy_table(table: &str) -> bool {
    PRESERVED_LEGACY_TABLES.contains(&table)
}

fn table_sql(conn: &Connection, table: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'table' AND name = ?1",
            params![table],
            |row| row.get(0),
        )
        .optional()?)
}

fn index_sql(conn: &Connection, index: &str) -> anyhow::Result<Option<String>> {
    Ok(conn
        .query_row(
            "SELECT sql FROM sqlite_schema WHERE type = 'index' AND name = ?1",
            params![index],
            |row| row.get(0),
        )
        .optional()?
        .flatten())
}

fn quote_identifier(identifier: &str) -> String {
    format!("\"{}\"", identifier.replace('"', "\"\""))
}

fn normalize_sql(sql: &str) -> String {
    let mut normalized = String::with_capacity(sql.len());
    let mut quote = None;
    let mut chars = sql.chars().peekable();
    while let Some(character) = chars.next() {
        if let Some(active_quote) = quote {
            normalized.push(character);
            if character == active_quote {
                if chars.peek() == Some(&active_quote) {
                    normalized.push(chars.next().expect("peeked quote"));
                } else {
                    quote = None;
                }
            }
            continue;
        }
        match character {
            '\'' | '"' => {
                quote = Some(character);
                normalized.push(character);
            }
            character if character.is_whitespace() => {}
            character => normalized.extend(character.to_lowercase()),
        }
    }
    normalized
}

fn extract_check_constraints(sql: &str) -> Vec<String> {
    let bytes = sql.as_bytes();
    let lower = sql.to_ascii_lowercase();
    let lower_bytes = lower.as_bytes();
    let mut checks = Vec::new();
    let mut cursor = 0;
    while cursor + 5 <= bytes.len() {
        let Some(relative) = lower[cursor..].find("check") else {
            break;
        };
        let start = cursor + relative;
        let before_is_identifier = start > 0
            && (lower_bytes[start - 1].is_ascii_alphanumeric() || lower_bytes[start - 1] == b'_');
        let after = start + 5;
        let after_is_identifier = after < bytes.len()
            && (lower_bytes[after].is_ascii_alphanumeric() || lower_bytes[after] == b'_');
        if before_is_identifier || after_is_identifier {
            cursor = after;
            continue;
        }
        let mut open = after;
        while open < bytes.len() && bytes[open].is_ascii_whitespace() {
            open += 1;
        }
        if open >= bytes.len() || bytes[open] != b'(' {
            cursor = after;
            continue;
        }
        let mut depth = 0_i64;
        let mut quote = None;
        let mut end = open;
        while end < bytes.len() {
            let byte = bytes[end];
            if let Some(active_quote) = quote {
                if byte == active_quote {
                    if end + 1 < bytes.len() && bytes[end + 1] == active_quote {
                        end += 2;
                        continue;
                    }
                    quote = None;
                }
            } else {
                match byte {
                    b'\'' | b'"' => quote = Some(byte),
                    b'(' => depth += 1,
                    b')' => {
                        depth -= 1;
                        if depth == 0 {
                            checks.push(normalize_sql(&sql[open + 1..end]));
                            end += 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            end += 1;
        }
        cursor = end.max(after);
    }
    checks
}

fn sql_where_clause(sql: &str) -> Option<String> {
    let lower = sql.to_ascii_lowercase();
    let index = lower.find("where")?;
    Some(normalize_sql(&sql[index + 5..]))
}
