import type { ApiClient } from "../api/client";
import { ConnectionsWorkspacePanel } from "./connections";

export type FlowLibraryPanelProps = { client: ApiClient | null };

/**
 * Compatibility entry point for the former Flow library placeholder. The
 * primary navigation now opens the real Connections control plane.
 */
export function FlowLibraryPanel({ client }: FlowLibraryPanelProps) {
  if (!client) {
    return (
      <div className="connections-page-state" role="status">
        <strong>Connections 尚未连接后端</strong>
        <span>服务恢复后将自动加载 Provider 和账号连接。</span>
      </div>
    );
  }
  return <ConnectionsWorkspacePanel client={client} />;
}
