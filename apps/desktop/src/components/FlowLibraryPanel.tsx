import type { ApiClient } from "../api/client";
import type { EnterprisePageHeaderChange } from "./enterprise/pageHeader";
import { ConnectionsWorkspacePanel } from "./connections";

export type FlowLibraryPanelProps = {
  client: ApiClient | null;
  onPageHeaderChange?: EnterprisePageHeaderChange;
};

/**
 * Compatibility entry point for the former Flow library placeholder. The
 * primary navigation now opens the real Connections control plane.
 */
export function FlowLibraryPanel({
  client,
  onPageHeaderChange,
}: FlowLibraryPanelProps) {
  if (!client) {
    return (
      <div className="connections-page-state" role="status">
        <strong>Connections 尚未连接后端</strong>
        <span>服务恢复后将自动加载 Provider 和账号连接。</span>
      </div>
    );
  }
  return (
    <ConnectionsWorkspacePanel
      client={client}
      onPageHeaderChange={onPageHeaderChange}
    />
  );
}
