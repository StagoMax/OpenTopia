import { useCallback, useEffect, useMemo, useState } from "react"
import {
  Activity,
  Bot,
  CheckCircle2,
  Circle,
  ClipboardList,
  FileCode2,
  GitBranch,
  Loader2,
  Plus,
  Send,
  Settings,
  ShieldAlert,
  TerminalSquare,
} from "lucide-react"
import { ApiClient, loadPlatformInfo } from "./api/client"
import type { AgentEvent, Message, MessagePart, PlatformInfo, Thread } from "./types"

type ServerStatus = "checking" | "online" | "offline"

export function App() {
  const [platform, setPlatform] = useState<PlatformInfo | null>(null)
  const [client, setClient] = useState<ApiClient | null>(null)
  const [serverStatus, setServerStatus] = useState<ServerStatus>("checking")
  const [serverError, setServerError] = useState<string | null>(null)
  const [threads, setThreads] = useState<Thread[]>([])
  const [activeThreadId, setActiveThreadId] = useState<string | null>(null)
  const [messages, setMessages] = useState<Message[]>([])
  const [events, setEvents] = useState<AgentEvent[]>([])
  const [composer, setComposer] = useState("")
  const [isSending, setIsSending] = useState(false)
  const [decidingApprovalId, setDecidingApprovalId] = useState<string | null>(null)
  const [settingsOpen, setSettingsOpen] = useState(false)

  const activeThread = useMemo(
    () => threads.find((thread) => thread.id === activeThreadId) ?? null,
    [threads, activeThreadId],
  )

  const ingestEvent = useCallback((event: AgentEvent) => {
    setEvents((current) => {
      if (current.some((item) => item.id === event.id)) return current
      return [...current, event].sort((a, b) => a.seq - b.seq)
    })

    if (event.payload.type === "assistant_message") {
      const assistantMessage = event.payload.message
      setMessages((current) => {
        if (current.some((message) => message.id === assistantMessage.id)) return current
        return [...current, assistantMessage]
      })
    }
  }, [])

  useEffect(() => {
    let cancelled = false
    void loadPlatformInfo().then(async (info) => {
      if (cancelled) return
      const nextClient = new ApiClient(info.backendUrl)
      setPlatform(info)
      setClient(nextClient)
      try {
        await nextClient.health()
        const loadedThreads = await nextClient.listThreads()
        if (cancelled) return
        setThreads(loadedThreads)
        setActiveThreadId((current) => current ?? loadedThreads[0]?.id ?? null)
        setServerStatus("online")
      } catch (error) {
        if (cancelled) return
        setServerStatus("offline")
        setServerError(error instanceof Error ? error.message : String(error))
      }
    })

    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!client || !activeThreadId) return
    let cancelled = false
    let source: EventSource | null = null

    void (async () => {
      const [loadedMessages, loadedEvents] = await Promise.all([
        client.listMessages(activeThreadId),
        client.listEvents(activeThreadId),
      ])
      if (cancelled) return
      setMessages(loadedMessages)
      setEvents(loadedEvents)
      const since = loadedEvents.at(-1)?.seq
      source = client.openEventStream(activeThreadId, since, ingestEvent)
    })().catch((error) => {
      if (!cancelled) setServerError(error instanceof Error ? error.message : String(error))
    })

    return () => {
      cancelled = true
      source?.close()
    }
  }, [activeThreadId, client, ingestEvent])

  async function createThread() {
    if (!client) return
    const thread = await client.createThread({ title: "OpenTopia MVP" })
    setThreads((current) => [thread, ...current])
    setActiveThreadId(thread.id)
  }

  async function submitMessage() {
    if (!client || !activeThread || !composer.trim() || isSending) return
    setIsSending(true)
    try {
      const message = await client.sendMessage(activeThread.id, composer.trim())
      setMessages((current) => [...current, message])
      setComposer("")
    } finally {
      setIsSending(false)
    }
  }

  async function decideApproval(approvalId: string, approved: boolean) {
    if (!client || !activeThread || decidingApprovalId) return
    setDecidingApprovalId(approvalId)
    try {
      await client.decideApproval(activeThread.id, approvalId, approved)
    } finally {
      setDecidingApprovalId(null)
    }
  }

  return (
    <div className="app-shell">
      <TopBar
        platform={platform}
        status={serverStatus}
        activeThread={activeThread}
        onSettings={() => setSettingsOpen(true)}
      />
      <main className="workspace">
        <Sidebar
          threads={threads}
          activeThreadId={activeThreadId}
          onSelect={setActiveThreadId}
          onNew={createThread}
        />
        <section className="center-pane">
          <ThreadHeader thread={activeThread} status={serverStatus} />
          {serverStatus === "offline" ? (
            <OfflineState backendUrl={platform?.backendUrl} error={serverError} />
          ) : activeThread ? (
            <>
              <MessageList messages={messages} events={events} />
              <Composer
                value={composer}
                isSending={isSending}
                onChange={setComposer}
                onSubmit={submitMessage}
              />
            </>
          ) : (
            <EmptyState onNew={createThread} />
          )}
        </section>
        <RightPanel
          thread={activeThread}
          events={events}
          decidingApprovalId={decidingApprovalId}
          onDecideApproval={decideApproval}
        />
      </main>
      {settingsOpen && <SettingsPanel platform={platform} onClose={() => setSettingsOpen(false)} />}
    </div>
  )
}

function TopBar({
  platform,
  status,
  activeThread,
  onSettings,
}: {
  platform: PlatformInfo | null
  status: ServerStatus
  activeThread: Thread | null
  onSettings(): void
}) {
  return (
    <header className="topbar">
      <div className="brand">
        <div className="brand-mark">O</div>
        <div>
          <strong>OpenTopia</strong>
          <span>AI Coding + Work Agent</span>
        </div>
      </div>
      <div className="topbar-meta">
        <StatusPill status={status} />
        <span>{platform?.os ?? "web"}</span>
        <span>{activeThread?.workspaceRoot ?? "No workspace"}</span>
        <button className="icon-button" aria-label="Settings" onClick={onSettings}>
          <Settings size={17} />
        </button>
      </div>
    </header>
  )
}

function SettingsPanel({ platform, onClose }: { platform: PlatformInfo | null; onClose(): void }) {
  return (
    <div className="modal-backdrop" role="presentation" onClick={onClose}>
      <section className="settings-panel" role="dialog" aria-modal="true" onClick={(event) => event.stopPropagation()}>
        <header>
          <h2>Settings</h2>
          <button className="secondary-button" onClick={onClose}>
            Close
          </button>
        </header>
        <div className="settings-grid">
          <label>
            Backend URL
            <code>{platform?.backendUrl ?? "http://127.0.0.1:8787"}</code>
          </label>
          <label>
            Platform
            <code>{platform?.os ?? "browser"}</code>
          </label>
          <label>
            Provider
            <span>Uses `OPENAI_API_KEY` / `OPENTOPIA_API_KEY` when present; otherwise mock provider.</span>
          </label>
          <label>
            Permission
            <span>Default `auto`; approvals are required for dangerous commands and can be allowed once.</span>
          </label>
        </div>
      </section>
    </div>
  )
}

function Sidebar({
  threads,
  activeThreadId,
  onSelect,
  onNew,
}: {
  threads: Thread[]
  activeThreadId: string | null
  onSelect(id: string): void
  onNew(): void
}) {
  return (
    <aside className="sidebar">
      <button className="new-thread" onClick={onNew}>
        <Plus size={16} />
        New Thread
      </button>
      <div className="sidebar-section">
        <span className="section-label">Threads</span>
        <div className="thread-list">
          {threads.map((thread) => (
            <button
              className={`thread-row ${thread.id === activeThreadId ? "active" : ""}`}
              key={thread.id}
              onClick={() => onSelect(thread.id)}
            >
              <GitBranch size={15} />
              <span>{thread.title}</span>
            </button>
          ))}
        </div>
      </div>
    </aside>
  )
}

function ThreadHeader({ thread, status }: { thread: Thread | null; status: ServerStatus }) {
  return (
    <div className="thread-header">
      <div>
        <h1>{thread?.title ?? "OpenTopia Workbench"}</h1>
        <p>{thread ? thread.workspaceRoot : "Create a thread to start an agent run."}</p>
      </div>
      <div className="thread-actions">
        <span className="mode-pill">Auto tools</span>
        <StatusPill status={status} />
      </div>
    </div>
  )
}

function MessageList({ messages, events }: { messages: Message[]; events: AgentEvent[] }) {
  return (
    <div className="message-list">
      {messages.length === 0 ? (
        <div className="empty-thread">
          <Bot size={42} />
          <h2>Ready for a coding task</h2>
          <p>The MVP agent can create turns, inspect the workspace root, emit tool events, and persist history.</p>
        </div>
      ) : (
        messages.map((message) => <MessageBubble key={message.id} message={message} />)
      )}
      {events.some((event) => event.payload.type === "model_delta") && (
        <div className="event-strip">
          <Activity size={14} />
          <span>{events.at(-1)?.payload.type === "turn_finished" ? "Turn complete" : "Agent is working"}</span>
        </div>
      )}
    </div>
  )
}

function MessageBubble({ message }: { message: Message }) {
  return (
    <article className={`message ${message.role}`}>
      <div className="message-avatar">{message.role === "user" ? "U" : "A"}</div>
      <div className="message-body">
        <div className="message-meta">
          <strong>{message.role === "user" ? "You" : "OpenTopia"}</strong>
          <span>{formatTime(message.createdAt)}</span>
        </div>
        {message.parts.map((part, index) => (
          <MessagePartView key={index} part={part} />
        ))}
      </div>
    </article>
  )
}

function MessagePartView({ part }: { part: MessagePart }) {
  if (part.type === "text") return <p className="message-text">{part.text}</p>
  if (part.type === "error") return <p className="message-error">{part.message}</p>
  if (part.type === "file_ref") return <code>{part.path}</code>
  if (part.type === "tool_call") return <pre>{JSON.stringify(part.call, null, 2)}</pre>
  return <pre>{part.result.output}</pre>
}

function Composer({
  value,
  isSending,
  onChange,
  onSubmit,
}: {
  value: string
  isSending: boolean
  onChange(value: string): void
  onSubmit(): void
}) {
  return (
    <div className="composer">
      <div className="command-hints">
        <span>/list</span>
        <span>/read README.md</span>
        <span>/write path</span>
        <span>/run cargo test</span>
        <span>/diff</span>
      </div>
      <textarea
        value={value}
        placeholder="Ask naturally, or use /read, /write, /run, /diff for deterministic local tools..."
        onChange={(event) => onChange(event.target.value)}
        onKeyDown={(event) => {
          if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
            event.preventDefault()
            onSubmit()
          }
        }}
      />
      <button className="send-button" disabled={isSending || !value.trim()} onClick={onSubmit}>
        {isSending ? <Loader2 size={17} className="spin" /> : <Send size={17} />}
        Send
      </button>
    </div>
  )
}

function RightPanel({
  thread,
  events,
  decidingApprovalId,
  onDecideApproval,
}: {
  thread: Thread | null
  events: AgentEvent[]
  decidingApprovalId: string | null
  onDecideApproval(approvalId: string, approved: boolean): void
}) {
  const latestToolResult = [...events].reverse().find((event) => event.payload.type === "tool_call_finished")
  const latestApproval = [...events].reverse().find((event) => event.payload.type === "approval_requested")
  const latestApprovalPayload =
    latestApproval?.payload.type === "approval_requested" ? latestApproval.payload : null
  return (
    <aside className="right-panel">
      <div className="panel-card">
        <div className="panel-title">
          <FileCode2 size={16} />
          Workspace
        </div>
        <p>{thread?.workspaceRoot ?? "No workspace selected."}</p>
      </div>
      <div className="panel-card timeline-card">
        <div className="panel-title">
          <TerminalSquare size={16} />
          Timeline
        </div>
        <div className="timeline">
          {events.length === 0 ? (
            <span className="muted">No events yet.</span>
          ) : (
            events.map((event) => <TimelineItem key={event.id} event={event} />)
          )}
        </div>
      </div>
      {latestApprovalPayload && (
        <div className="panel-card approval-card">
          <div className="panel-title">
            <ShieldAlert size={16} />
            Approval Needed
          </div>
          <p>{latestApprovalPayload.reason}</p>
          <code>{latestApprovalPayload.action}</code>
          <div className="approval-actions">
            <button
              className="secondary-button"
              disabled={decidingApprovalId === latestApprovalPayload.approval_id}
              onClick={() => onDecideApproval(latestApprovalPayload.approval_id, false)}
            >
              Deny
            </button>
            <button
              className="primary-button"
              disabled={decidingApprovalId === latestApprovalPayload.approval_id}
              onClick={() => onDecideApproval(latestApprovalPayload.approval_id, true)}
            >
              Allow Once
            </button>
          </div>
        </div>
      )}
      <div className="panel-card">
        <div className="panel-title">
          <ClipboardList size={16} />
          Latest Tool Output
        </div>
        {latestToolResult?.payload.type === "tool_call_finished" ? (
          <pre className="tool-output">{latestToolResult.payload.result.output}</pre>
        ) : (
          <p>No tool output yet.</p>
        )}
      </div>
    </aside>
  )
}

function TimelineItem({ event }: { event: AgentEvent }) {
  const title = eventTitle(event)
  const done = event.payload.type === "turn_finished" || event.payload.type === "tool_call_finished"
  return (
    <div className="timeline-item">
      {done ? <CheckCircle2 size={15} /> : <Circle size={15} />}
      <div>
        <strong>{title}</strong>
        <span>{formatTime(event.createdAt)}</span>
      </div>
    </div>
  )
}

function EmptyState({ onNew }: { onNew(): void }) {
  return (
    <div className="empty-state">
      <Bot size={48} />
      <h2>Create your first thread</h2>
      <p>OpenTopia will store every message, tool call, and event for replay and audit.</p>
      <button className="new-thread large" onClick={onNew}>
        <Plus size={16} />
        New Thread
      </button>
    </div>
  )
}

function OfflineState({ backendUrl, error }: { backendUrl?: string; error: string | null }) {
  return (
    <div className="empty-state offline">
      <TerminalSquare size={48} />
      <h2>Local server is offline</h2>
      <p>Start the Rust server, then reload the desktop app.</p>
      <code>cargo run -p opentopia-server</code>
      <small>{backendUrl ?? "http://127.0.0.1:8787"}</small>
      {error && <pre>{error}</pre>}
    </div>
  )
}

function StatusPill({ status }: { status: ServerStatus }) {
  return <span className={`status-pill ${status}`}>{status}</span>
}

function eventTitle(event: AgentEvent): string {
  switch (event.payload.type) {
    case "turn_started":
      return "Turn started"
    case "model_delta":
      return "Model streamed text"
    case "tool_call_started":
      return `Tool started: ${event.payload.call.name}`
    case "tool_call_finished":
      return "Tool finished"
    case "assistant_message":
      return "Assistant message"
    case "file_changed":
      return `File changed: ${event.payload.path}`
    case "approval_requested":
      return "Approval requested"
    case "turn_finished":
      return "Turn finished"
    case "error":
      return "Agent error"
  }
}

function formatTime(value: string): string {
  return new Date(value).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" })
}
