//! Process-local document handles and derived tabular selections.
//!
//! The handle binds an original resource to its owning thread. Operations pass
//! only compact IDs; full rows stay in this layer and never need to round-trip
//! through the model or an intermediate file.

use super::DocumentResourceRef;
use crate::spreadsheet::SpreadsheetCellInput;
use anyhow::Context;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};
use uuid::Uuid;

const MAX_DOCUMENT_SESSIONS: usize = 256;
const MAX_SELECTIONS: usize = 512;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum DocumentOpenMode {
    Read,
    Edit,
    Create,
}

impl Default for DocumentOpenMode {
    fn default() -> Self {
        Self::Read
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DocumentKind {
    Spreadsheet,
}

impl DocumentKind {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Spreadsheet => "spreadsheet",
        }
    }
}

#[derive(Debug, Clone)]
pub(super) struct DocumentSession {
    pub(super) id: Uuid,
    pub(super) owner_thread_id: Option<Uuid>,
    pub(super) kind: DocumentKind,
    pub(super) mode: DocumentOpenMode,
    pub(super) resource: DocumentResourceRef,
    pub(super) disclosed_operations: BTreeSet<String>,
}

impl DocumentSession {
    pub(super) fn is_editable(&self) -> bool {
        matches!(self.mode, DocumentOpenMode::Edit | DocumentOpenMode::Create)
    }
}

#[derive(Debug, Clone)]
pub(super) struct TabularSelection {
    pub(super) id: Uuid,
    pub(super) owner_thread_id: Option<Uuid>,
    pub(super) source_document_id: Uuid,
    pub(super) rows: Vec<Vec<SpreadsheetCellInput>>,
    pub(super) source_rows: Vec<u32>,
    pub(super) source_columns: Vec<Option<u32>>,
}

impl TabularSelection {
    pub(super) fn width(&self) -> usize {
        self.source_columns.len()
    }
}

#[derive(Default)]
struct DocumentSessionStore {
    documents: HashMap<Uuid, DocumentSession>,
    document_order: VecDeque<Uuid>,
    selections: HashMap<Uuid, TabularSelection>,
    selection_order: VecDeque<Uuid>,
}

fn store() -> &'static Mutex<DocumentSessionStore> {
    static STORE: OnceLock<Mutex<DocumentSessionStore>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(DocumentSessionStore::default()))
}

fn lock_store() -> anyhow::Result<std::sync::MutexGuard<'static, DocumentSessionStore>> {
    store()
        .lock()
        .map_err(|_| anyhow::anyhow!("document session store is unavailable"))
}

pub(super) fn insert_document(
    owner_thread_id: Option<Uuid>,
    kind: DocumentKind,
    mode: DocumentOpenMode,
    resource: DocumentResourceRef,
) -> anyhow::Result<DocumentSession> {
    let mut state = lock_store()?;
    while state.documents.len() >= MAX_DOCUMENT_SESSIONS {
        let Some(expired) = state.document_order.pop_front() else {
            break;
        };
        state.documents.remove(&expired);
        let selection_ids = state
            .selections
            .values()
            .filter(|selection| selection.source_document_id == expired)
            .map(|selection| selection.id)
            .collect::<Vec<_>>();
        for selection_id in selection_ids {
            state.selections.remove(&selection_id);
            state.selection_order.retain(|id| *id != selection_id);
        }
    }
    let session = DocumentSession {
        id: Uuid::new_v4(),
        owner_thread_id,
        kind,
        mode,
        resource,
        disclosed_operations: BTreeSet::new(),
    };
    state.document_order.push_back(session.id);
    state.documents.insert(session.id, session.clone());
    Ok(session)
}

pub(super) fn get_document(
    document_id: Uuid,
    owner_thread_id: Option<Uuid>,
) -> anyhow::Result<DocumentSession> {
    let state = lock_store()?;
    let session = state
        .documents
        .get(&document_id)
        .with_context(|| format!("document {document_id} is not open"))?;
    anyhow::ensure!(
        session.owner_thread_id == owner_thread_id,
        "document {document_id} belongs to another thread"
    );
    Ok(session.clone())
}

pub(super) fn get_document_unscoped(document_id: Uuid) -> anyhow::Result<DocumentSession> {
    let state = lock_store()?;
    state
        .documents
        .get(&document_id)
        .cloned()
        .with_context(|| format!("document {document_id} is not open"))
}

pub(super) fn disclose_operations(
    document_id: Uuid,
    owner_thread_id: Option<Uuid>,
    operations: impl IntoIterator<Item = String>,
) -> anyhow::Result<DocumentSession> {
    let mut state = lock_store()?;
    let session = state
        .documents
        .get_mut(&document_id)
        .with_context(|| format!("document {document_id} is not open"))?;
    anyhow::ensure!(
        session.owner_thread_id == owner_thread_id,
        "document {document_id} belongs to another thread"
    );
    session.disclosed_operations.extend(operations);
    Ok(session.clone())
}

pub(super) fn require_disclosed_operation(
    session: &DocumentSession,
    operation: &str,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        session.disclosed_operations.contains(operation),
        "operation `{operation}` has not been loaded for document {}; call document_get_operation_schemas first",
        session.id
    );
    Ok(())
}

pub(super) fn insert_selection(
    owner_thread_id: Option<Uuid>,
    source_document_id: Uuid,
    rows: Vec<Vec<SpreadsheetCellInput>>,
    source_rows: Vec<u32>,
    source_columns: Vec<Option<u32>>,
) -> anyhow::Result<TabularSelection> {
    let mut state = lock_store()?;
    anyhow::ensure!(
        rows.iter().all(|row| row.len() == source_columns.len()),
        "selection rows must have a consistent width"
    );
    anyhow::ensure!(
        rows.len() == source_rows.len(),
        "selection row provenance must match its data"
    );
    while state.selections.len() >= MAX_SELECTIONS {
        let Some(expired) = state.selection_order.pop_front() else {
            break;
        };
        state.selections.remove(&expired);
    }
    let selection = TabularSelection {
        id: Uuid::new_v4(),
        owner_thread_id,
        source_document_id,
        rows,
        source_rows,
        source_columns,
    };
    state.selection_order.push_back(selection.id);
    state.selections.insert(selection.id, selection.clone());
    Ok(selection)
}

pub(super) fn get_selection(
    selection_id: Uuid,
    owner_thread_id: Option<Uuid>,
) -> anyhow::Result<TabularSelection> {
    let state = lock_store()?;
    let selection = state
        .selections
        .get(&selection_id)
        .with_context(|| format!("selection {selection_id} does not exist"))?;
    anyhow::ensure!(
        selection.owner_thread_id == owner_thread_id,
        "selection {selection_id} belongs to another thread"
    );
    Ok(selection.clone())
}
