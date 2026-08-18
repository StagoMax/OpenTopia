use opentopia_core::collaboration::AgentThreadId;
use opentopia_core::collaboration::RuntimeSnapshotV1;
use opentopia_core::{
    AgentActivityEnvelopeV1, AgentActivityNotification, AgentEvent, AgentEventEnvelopeV1,
    AgentEventPayload, DesktopStreamEnvelope, DesktopStreamKind, TerminalEvent,
    TerminalEventEnvelopeV1, TerminalEventKind,
};
use schemars::{schema::RootSchema, schema_for};
use serde::Serialize;
use serde_json::Value;
use std::env;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;

const CONTRACTS: &[(&str, fn() -> RootSchema)] = &[
    ("agent-event-envelope-v1.schema.json", agent_event_schema),
    (
        "agent-activity-envelope-v1.schema.json",
        agent_activity_schema,
    ),
    (
        "terminal-event-envelope-v1.schema.json",
        terminal_event_schema,
    ),
    ("runtime-snapshot-v1.schema.json", runtime_snapshot_schema),
];

fn main() -> anyhow::Result<()> {
    let (output_dir, check) = parse_args()?;
    if !check {
        fs::create_dir_all(&output_dir)?;
    }

    let mut stale = Vec::new();
    for (name, schema) in CONTRACTS {
        let encoded = encode_schema(&schema())?;
        let path = output_dir.join(name);
        write_or_check(path, encoded, check, &mut stale)?;
    }
    let fixtures = encode_canonical_json(&stream_fixtures())?;
    write_or_check(
        output_dir.join("stream-contract-v1.fixtures.json"),
        fixtures,
        check,
        &mut stale,
    )?;

    if stale.is_empty() {
        return Ok(());
    }
    anyhow::bail!(
        "generated Desktop API contracts are stale: {}",
        stale
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    )
}

fn encode_schema(value: &impl Serialize) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(value)?;
    normalize_schema(&mut value);
    encode_json_value(&value)
}

fn encode_canonical_json(value: &impl Serialize) -> anyhow::Result<String> {
    let mut value = serde_json::to_value(value)?;
    canonicalize_json(&mut value);
    encode_json_value(&value)
}

fn encode_json_value(value: &Value) -> anyhow::Result<String> {
    let mut encoded = serde_json::to_string_pretty(value)?;
    encoded.push('\n');
    Ok(encoded)
}

fn normalize_schema(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                normalize_schema(item);
            }
        }
        Value::Object(object) => {
            // Schemars evaluates Serde default functions while deriving a schema. A generated
            // UUID is not a meaningful JSON Schema default and makes the artifact change on every
            // run, whereas deterministic defaults remain useful to the legacy wire decoder.
            if object.get("format").and_then(Value::as_str) == Some("uuid") {
                object.remove("default");
            }
            for child in object.values_mut() {
                normalize_schema(child);
            }
            object.sort_keys();
        }
        _ => {}
    }
}

fn canonicalize_json(value: &mut Value) {
    match value {
        Value::Array(items) => {
            for item in items {
                canonicalize_json(item);
            }
        }
        Value::Object(object) => {
            for child in object.values_mut() {
                canonicalize_json(child);
            }
            object.sort_keys();
        }
        _ => {}
    }
}

fn write_or_check(
    path: PathBuf,
    encoded: String,
    check: bool,
    stale: &mut Vec<PathBuf>,
) -> anyhow::Result<()> {
    if check {
        if fs::read_to_string(&path).ok().as_deref() != Some(encoded.as_str()) {
            stale.push(path);
        }
    } else {
        fs::write(path, encoded)?;
    }
    Ok(())
}

fn parse_args() -> anyhow::Result<(PathBuf, bool)> {
    let mut output_dir = None;
    let mut check = false;
    let mut args = env::args().skip(1);
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--output" => output_dir = args.next().map(PathBuf::from),
            "--check" => check = true,
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    let output_dir =
        output_dir.ok_or_else(|| anyhow::anyhow!("--output <directory> is required"))?;
    Ok((output_dir, check))
}

fn agent_event_schema() -> RootSchema {
    schema_for!(AgentEventEnvelopeV1)
}

fn agent_activity_schema() -> RootSchema {
    schema_for!(AgentActivityEnvelopeV1)
}

fn terminal_event_schema() -> RootSchema {
    schema_for!(TerminalEventEnvelopeV1)
}

fn runtime_snapshot_schema() -> RootSchema {
    schema_for!(RuntimeSnapshotV1)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct StreamFixtures {
    agent_event: AgentEventEnvelopeV1,
    agent_activity: AgentActivityEnvelopeV1,
    terminal_event: TerminalEventEnvelopeV1,
}

fn stream_fixtures() -> StreamFixtures {
    let thread_id = uuid("00000000-0000-4000-8000-000000000002");
    let agent_event = AgentEvent {
        id: uuid("00000000-0000-4000-8000-000000000001"),
        thread_id,
        turn_id: None,
        seq: 7,
        created_at: "2026-08-17T00:00:00Z".parse().expect("fixture datetime"),
        payload: AgentEventPayload::ModelDelta {
            text: "hello".to_string(),
        },
    };
    let agent_activity = AgentActivityNotification {
        seq: 9,
        agent_thread_id: AgentThreadId::from_uuid(uuid("00000000-0000-4000-8000-000000000003")),
    };
    let terminal_event = TerminalEvent {
        id: uuid("00000000-0000-4000-8000-000000000004"),
        thread_id,
        command_id: uuid("00000000-0000-4000-8000-000000000005"),
        seq: 10,
        created_at: "2026-08-17T00:00:00Z".parse().expect("fixture datetime"),
        kind: TerminalEventKind::Stdout,
        command: None,
        cwd: None,
        data: Some("ok".to_string()),
        exit_code: None,
        success: None,
        message: None,
    };

    StreamFixtures {
        agent_event: DesktopStreamEnvelope::new(
            DesktopStreamKind::AgentEvent,
            agent_event.seq,
            agent_event,
        ),
        agent_activity: DesktopStreamEnvelope::new(
            DesktopStreamKind::AgentActivity,
            agent_activity.seq,
            agent_activity,
        ),
        terminal_event: DesktopStreamEnvelope::new(
            DesktopStreamKind::TerminalEvent,
            i64::try_from(terminal_event.seq).expect("fixture sequence"),
            terminal_event,
        ),
    }
}

fn uuid(value: &str) -> Uuid {
    Uuid::parse_str(value).expect("fixture UUID")
}
