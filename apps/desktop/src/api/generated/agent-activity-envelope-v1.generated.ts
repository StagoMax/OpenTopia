/* eslint-disable */
// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.

export type DesktopStreamKind = "agent_event" | "agent_activity" | "terminal_event";

export interface AgentActivityEnvelopeV1 {
  apiVersion: number;
  data: AgentActivityNotification;
  kind: DesktopStreamKind;
  seq: number;
  [k: string]: unknown;
}
export interface AgentActivityNotification {
  agentThreadId: string;
  seq: number;
  [k: string]: unknown;
}
