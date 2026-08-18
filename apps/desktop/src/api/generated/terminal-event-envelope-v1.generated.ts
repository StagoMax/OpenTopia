/* eslint-disable */
// Generated from the Rust DTO schema. Run `pnpm contracts:generate`; do not edit.

export type TerminalEventKind =
  "started" | "stdout" | "stderr" | "finished" | "cancelled" | "error";
export type DesktopStreamKind = "agent_event" | "agent_activity" | "terminal_event";

export interface TerminalEventEnvelopeV1 {
  apiVersion: number;
  data: TerminalEvent;
  kind: DesktopStreamKind;
  seq: number;
  [k: string]: unknown;
}
export interface TerminalEvent {
  command?: string | null;
  commandId: string;
  createdAt: string;
  cwd?: string | null;
  data?: string | null;
  exitCode?: number | null;
  id: string;
  message?: string | null;
  seq: number;
  success?: boolean | null;
  threadId: string;
  type: TerminalEventKind;
  [k: string]: unknown;
}
