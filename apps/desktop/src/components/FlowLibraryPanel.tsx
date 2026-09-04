import type { ApiClient } from "../api/client";
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
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
  const { t } = useApplicationLanguage();
  if (!client) {
    return (
      <div className="connections-page-state" role="status">
        <strong>{t("flow.connection.workspace.backendUnavailable")}</strong>
        <span>{t("flow.connection.workspace.backendRestore")}</span>
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
