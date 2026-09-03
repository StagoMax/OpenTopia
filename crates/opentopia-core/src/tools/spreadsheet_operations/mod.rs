//! Atomic spreadsheet operations used by the generic document protocol.
//!
//! Every registry entry owns its name, argument schema, effect classification,
//! edit precondition, and handler. Composite workflows are deliberately absent;
//! they are composed through server-side selection handles.

mod backend;
mod contracts;
mod selection;

use super::document_session::{DocumentKind, DocumentSession};
use super::ToolInvocationContext;
use crate::model::ToolResult;
use async_trait::async_trait;
use serde_json::Value;
use std::sync::OnceLock;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OperationEffect {
    Observation,
    SessionMutation,
    FileMutation,
}

#[async_trait]
pub(super) trait DocumentOperationHandler: Send + Sync {
    fn name(&self) -> &'static str;
    fn version(&self) -> &'static str {
        "1"
    }
    fn description(&self) -> &'static str;
    fn effect(&self) -> OperationEffect;
    fn requires_editable_document(&self) -> bool;
    fn arguments_schema(&self) -> Value;

    async fn execute(
        &self,
        call_id: Uuid,
        session: &DocumentSession,
        arguments: Value,
        ctx: ToolInvocationContext,
    ) -> anyhow::Result<ToolResult>;
}

fn spreadsheet_handlers() -> &'static Vec<Box<dyn DocumentOperationHandler>> {
    static HANDLERS: OnceLock<Vec<Box<dyn DocumentOperationHandler>>> = OnceLock::new();
    HANDLERS.get_or_init(|| {
        let mut handlers = backend::handlers();
        handlers.extend(selection::handlers());
        handlers
    })
}

pub(super) fn handlers_for(kind: DocumentKind) -> &'static [Box<dyn DocumentOperationHandler>] {
    match kind {
        DocumentKind::Spreadsheet => spreadsheet_handlers().as_slice(),
    }
}

pub(super) fn handler_for(
    kind: DocumentKind,
    name: &str,
) -> Option<&'static dyn DocumentOperationHandler> {
    handlers_for(kind)
        .iter()
        .find(|handler| handler.name() == name)
        .map(Box::as_ref)
}

pub(super) fn available_operation_names(session: &DocumentSession) -> Vec<&'static str> {
    handlers_for(session.kind)
        .iter()
        .filter(|handler| !handler.requires_editable_document() || session.is_editable())
        .map(|handler| handler.name())
        .collect()
}
