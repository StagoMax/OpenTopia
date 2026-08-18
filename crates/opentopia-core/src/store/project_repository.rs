use super::sqlite_rows::{encode_model_selection, map_project, map_thread};
use super::StoreError;
use crate::model::{Project, Thread};
use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use uuid::Uuid;

pub(super) fn table_has_column(
    conn: &Connection,
    table: &str,
    column: &str,
) -> anyhow::Result<bool> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;
    while let Some(row) = rows.next()? {
        if row.get::<_, String>(1)? == column {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(super) fn backfill_thread_projects(conn: &mut Connection) -> anyhow::Result<()> {
    let tx = conn.transaction()?;
    let mut projects_by_key = HashMap::new();
    {
        let mut stmt =
            tx.prepare("SELECT id, workspace_key FROM projects WHERE workspace_key IS NOT NULL")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (id, key) = row?;
            projects_by_key.insert(key, id);
        }
    }

    let mut threads = Vec::new();
    {
        let mut stmt = tx.prepare(
            r#"
            SELECT id, workspace_root, created_at, updated_at
            FROM threads
            WHERE project_id IS NULL
            ORDER BY created_at ASC
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
            threads.push(row?);
        }
    }

    for (thread_id, workspace_root, created_at, updated_at) in threads {
        let workspace_key = normalize_workspace_key(Path::new(&workspace_root));
        let project_id = if let Some(project_id) = projects_by_key.get(&workspace_key) {
            project_id.clone()
        } else {
            let project_id = Uuid::new_v4().to_string();
            tx.execute(
                r#"
                INSERT INTO projects (
                    id, name, workspace_root, workspace_key, pinned, sort_order,
                    created_at, updated_at
                )
                VALUES (
                    ?1, ?2, ?3, ?4, 0,
                    (SELECT COALESCE(MAX(sort_order), -1) + 1 FROM projects),
                    ?5, ?6
                )
                "#,
                params![
                    &project_id,
                    project_name_from_workspace(&workspace_root),
                    &workspace_root,
                    &workspace_key,
                    &created_at,
                    &updated_at,
                ],
            )?;
            projects_by_key.insert(workspace_key, project_id.clone());
            project_id
        };
        tx.execute(
            "UPDATE threads SET project_id = ?1 WHERE id = ?2",
            params![project_id, thread_id],
        )?;
    }
    tx.commit()?;
    Ok(())
}

pub fn normalize_workspace_key(path: &Path) -> String {
    let original = path.to_string_lossy();
    let had_backslash = original.contains('\\');
    let mut value = original.trim().replace('\\', "/");
    let lowercase = value.to_ascii_lowercase();
    let mut is_windows = had_backslash;

    if lowercase.starts_with("//?/unc/") {
        value = format!("//{}", &value[8..]);
        is_windows = true;
    } else if lowercase.starts_with("//?/") {
        value = value[4..].to_string();
        is_windows = true;
    }

    let bytes = value.as_bytes();
    let has_drive = bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let is_unc = value.starts_with("//");
    let is_absolute = !is_unc && value.starts_with('/');
    let drive_absolute = has_drive && value.as_bytes().get(2) == Some(&b'/');
    is_windows |= has_drive || is_unc;

    let minimum_depth = if is_unc {
        2
    } else if drive_absolute {
        1
    } else {
        0
    };
    let mut segments: Vec<&str> = Vec::new();
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        match segment {
            "." => {}
            ".." if segments.len() > minimum_depth && segments.last() != Some(&"..") => {
                segments.pop();
            }
            ".." if !is_absolute && !drive_absolute && !is_unc => segments.push(segment),
            ".." => {}
            _ => segments.push(segment),
        }
    }

    let mut normalized = if is_unc {
        format!("//{}", segments.join("/"))
    } else if is_absolute {
        format!("/{}", segments.join("/"))
    } else {
        segments.join("/")
    };
    if drive_absolute && segments.len() == 1 {
        normalized.push('/');
    }
    if normalized.is_empty() && !original.trim().is_empty() {
        normalized.push('.');
    }
    if is_windows {
        normalized.make_ascii_lowercase();
    }
    normalized
}

pub(super) fn project_name_from_workspace(workspace_root: &str) -> String {
    let normalized = workspace_root.trim().replace('\\', "/");
    normalized
        .trim_end_matches('/')
        .rsplit('/')
        .find(|part| !part.is_empty())
        .filter(|part| *part != ".")
        .unwrap_or("Workspace")
        .to_string()
}

pub(super) fn validated_project_name(name: String) -> anyhow::Result<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err(StoreError::EmptyProjectName.into());
    }
    Ok(name.to_string())
}

pub(super) fn validated_workspace_key(workspace_root: &Path) -> anyhow::Result<String> {
    let key = normalize_workspace_key(workspace_root);
    if key.is_empty() {
        return Err(StoreError::EmptyWorkspaceRoot.into());
    }
    Ok(key)
}

pub(super) fn project_workspace_values(
    workspace_root: &Option<PathBuf>,
) -> anyhow::Result<(Option<String>, Option<String>)> {
    workspace_root
        .as_ref()
        .map(|path| {
            Ok((
                Some(path.to_string_lossy().into_owned()),
                Some(validated_workspace_key(path)?),
            ))
        })
        .unwrap_or(Ok((None, None)))
}

pub(super) fn ensure_workspace_available(
    conn: &Connection,
    workspace_key: Option<&str>,
    exclude_project_id: Option<Uuid>,
) -> anyhow::Result<()> {
    let Some(workspace_key) = workspace_key else {
        return Ok(());
    };
    let existing: Option<String> = conn
        .query_row(
            "SELECT id FROM projects WHERE workspace_key = ?1",
            params![workspace_key],
            |row| row.get(0),
        )
        .optional()?;
    if existing.as_deref()
        != exclude_project_id
            .as_ref()
            .map(|id| id.to_string())
            .as_deref()
        && existing.is_some()
    {
        return Err(StoreError::DuplicateWorkspace(workspace_key.to_string()).into());
    }
    Ok(())
}

pub(super) fn insert_project(
    conn: &Connection,
    project: &Project,
    workspace_root: Option<&str>,
    workspace_key: Option<&str>,
) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO projects (
            id, name, workspace_root, workspace_key, pinned, sort_order,
            created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
        "#,
        params![
            project.id.to_string(),
            &project.name,
            workspace_root,
            workspace_key,
            project.pinned as i64,
            project.sort_order,
            project.created_at.to_rfc3339(),
            project.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(super) fn query_project(conn: &Connection, id: Uuid) -> anyhow::Result<Option<Project>> {
    conn.query_row(
        r#"
        SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
        FROM projects
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_project,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn query_project_by_workspace_key(
    conn: &Connection,
    workspace_key: &str,
) -> anyhow::Result<Option<Project>> {
    conn.query_row(
        r#"
        SELECT id, name, workspace_root, pinned, sort_order, created_at, updated_at
        FROM projects
        WHERE workspace_key = ?1
        "#,
        params![workspace_key],
        map_project,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn insert_thread(conn: &Connection, thread: &Thread) -> anyhow::Result<()> {
    conn.execute(
        r#"
        INSERT INTO threads (
            id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
        )
        VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
        "#,
        params![
            thread.id.to_string(),
            &thread.title,
            thread.workspace_root.to_string_lossy(),
            thread.project_id.map(|id| id.to_string()),
            thread.archived_at.map(|value| value.to_rfc3339()),
            thread.experience_mode.as_str(),
            encode_model_selection(thread.model_selection.as_ref())?,
            thread.created_at.to_rfc3339(),
            thread.updated_at.to_rfc3339(),
        ],
    )?;
    Ok(())
}

pub(super) fn query_thread(conn: &Connection, id: Uuid) -> anyhow::Result<Option<Thread>> {
    conn.query_row(
        r#"
        SELECT id, title, workspace_root, project_id, archived_at, experience_mode, model_selection, created_at, updated_at
        FROM threads
        WHERE id = ?1
        "#,
        params![id.to_string()],
        map_thread,
    )
    .optional()
    .map_err(Into::into)
}

pub(super) fn touch_thread(conn: &Connection, thread_id: Uuid) -> anyhow::Result<()> {
    conn.execute(
        "UPDATE threads SET updated_at = ?1 WHERE id = ?2",
        params![Utc::now().to_rfc3339(), thread_id.to_string()],
    )?;
    Ok(())
}
