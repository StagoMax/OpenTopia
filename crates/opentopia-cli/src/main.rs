use clap::{Parser, Subcommand};
use opentopia_core::{
    AgentCore, AgentEvent, AgentTurnInput, Message, MessageRole, PermissionMode, SessionStore,
    SqliteSessionStore,
};
use std::path::PathBuf;
use std::sync::Arc;
use uuid::Uuid;

#[derive(Debug, Parser)]
#[command(name = "opentopia")]
#[command(about = "OpenTopia local-first AI Coding + Work Agent CLI")]
struct Args {
    #[arg(long, env = "OPENTOPIA_DB", default_value = ".opentopia/opentopia.db")]
    db: PathBuf,
    #[arg(long, env = "OPENTOPIA_PERMISSION", default_value = "auto")]
    permission: PermissionMode,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Threads,
    New {
        #[arg(long)]
        title: Option<String>,
        #[arg(long, default_value = ".")]
        workspace: PathBuf,
    },
    Send {
        thread_id: Uuid,
        content: String,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let store = Arc::new(SqliteSessionStore::open(args.db)?);

    match args.command {
        Command::Threads => {
            println!("{}", serde_json::to_string_pretty(&store.list_threads()?)?);
        }
        Command::New { title, workspace } => {
            let workspace = workspace.canonicalize().unwrap_or(workspace);
            let thread = store.create_thread(title, workspace)?;
            println!("{}", serde_json::to_string_pretty(&thread)?);
        }
        Command::Send { thread_id, content } => {
            let thread = store
                .get_thread(thread_id)?
                .ok_or_else(|| anyhow::anyhow!("thread not found: {thread_id}"))?;
            let user_message = store.append_message(Message::text(
                thread_id,
                MessageRole::User,
                content.clone(),
            ))?;

            let agent = AgentCore::from_env();
            let turn_id = Uuid::new_v4();
            let payloads = agent
                .run_turn(AgentTurnInput {
                    thread_id,
                    user_message_id: user_message.id,
                    workspace_root: thread.workspace_root,
                    content,
                    user_content: Vec::new(),
                    context_summary: None,
                    conversation: Vec::new(),
                    permission_mode: args.permission,
                    context_budget: None,
                    provider_cursor: None,
                    store: Some(store.clone() as Arc<dyn SessionStore>),
                    cancellation: None,
                })
                .await?;

            let mut events = Vec::new();
            for payload in payloads {
                if let opentopia_core::AgentEventPayload::AssistantMessage { message } = &payload {
                    store.append_message(message.clone())?;
                }
                events.push(store.append_event(AgentEvent::new(
                    thread_id,
                    Some(turn_id),
                    0,
                    payload,
                ))?);
            }
            println!("{}", serde_json::to_string_pretty(&events)?);
        }
    }

    Ok(())
}
