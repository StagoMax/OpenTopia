import type { AgentEvent, Message, PlatformInfo, Thread } from "../types"

export class ApiClient {
  constructor(private readonly baseUrl: string) {}

  async health(): Promise<{ ok: boolean; service: string }> {
    return this.get("/health")
  }

  async listThreads(): Promise<Thread[]> {
    return this.get("/api/threads")
  }

  async createThread(input: { title?: string; workspaceRoot?: string }): Promise<Thread> {
    return this.post("/api/threads", input)
  }

  async listMessages(threadId: string): Promise<Message[]> {
    return this.get(`/api/threads/${threadId}/messages`)
  }

  async sendMessage(threadId: string, content: string): Promise<Message> {
    return this.post(`/api/threads/${threadId}/messages`, { content })
  }

  async listEvents(threadId: string, since?: number): Promise<AgentEvent[]> {
    const query = since ? `?since=${since}` : ""
    return this.get(`/api/threads/${threadId}/events${query}`)
  }

  async decideApproval(threadId: string, approvalId: string, approved: boolean): Promise<{ accepted: boolean; executed: boolean }> {
    return this.post(`/api/threads/${threadId}/approvals/${approvalId}/decision`, { approved })
  }

  openEventStream(threadId: string, since: number | undefined, onEvent: (event: AgentEvent) => void): EventSource {
    const query = since ? `?since=${since}` : ""
    const source = new EventSource(`${this.baseUrl}/api/threads/${threadId}/events/stream${query}`)
    source.onmessage = (message) => {
      onEvent(JSON.parse(message.data) as AgentEvent)
    }
    source.addEventListener("turn_started", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("model_delta", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("tool_call_started", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("tool_call_finished", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("assistant_message", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("turn_finished", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    source.addEventListener("agent_error", (message) => onEvent(JSON.parse((message as MessageEvent).data)))
    return source
  }

  private async get<T>(path: string): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`)
    return parseResponse<T>(response)
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await fetch(`${this.baseUrl}${path}`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
    })
    return parseResponse<T>(response)
  }
}

export async function loadPlatformInfo(): Promise<PlatformInfo> {
  if (window.opentopia) return window.opentopia.getPlatformInfo()
  return {
    platform: "web",
    backendUrl: import.meta.env.VITE_OPENTOPIA_SERVER_URL || "http://127.0.0.1:8787",
  }
}

async function parseResponse<T>(response: Response): Promise<T> {
  if (!response.ok) {
    const text = await response.text()
    throw new Error(text || `${response.status} ${response.statusText}`)
  }
  return response.json() as Promise<T>
}
