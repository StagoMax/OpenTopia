import type { ApiClient } from "../../api/client";
import type { AppSettings } from "../../types";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { FlowLibraryPanel } from "../FlowLibraryPanel";
import { LibraryPanel } from "../LibraryPanel";
import { AgentsPage } from "./AgentsPage";
import { FlowInboxPage } from "./FlowInboxPage";
import { OverviewPage } from "./OverviewPage";
import { RunsPage } from "./RunsPage";
import { TrustPage } from "./TrustPage";
import { WorkflowTemplatesPage } from "./WorkflowTemplatesPage";
import type { EnterprisePageHeaderChange } from "./pageHeader";
import "./enterprise.css";

export function FlowEnterpriseWorkspace({
  client,
  settings,
  threadId,
  view,
  workspaceRoot,
  onNavigate,
  onPageHeaderChange,
}: {
  client: ApiClient;
  settings: AppSettings | null;
  threadId: string | null;
  view: Exclude<FlowPrimaryView, "conversation">;
  workspaceRoot: string | null;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
  onPageHeaderChange?: EnterprisePageHeaderChange;
}) {
  if (view === "overview")
    return <OverviewPage client={client} onNavigate={onNavigate} />;
  if (view === "agents")
    return (
      <AgentsPage
        client={client}
        settings={settings}
        threadId={threadId}
        workspaceRoot={workspaceRoot}
        onPageHeaderChange={onPageHeaderChange}
      />
    );
  if (view === "workflow-templates")
    return (
      <WorkflowTemplatesPage
        client={client}
        onPageHeaderChange={onPageHeaderChange}
        threadId={threadId}
      />
    );
  if (view === "inbox") return <FlowInboxPage client={client} />;
  if (view === "runs") return <RunsPage client={client} />;
  if (view === "connections")
    return (
      <FlowLibraryPanel
        client={client}
        onPageHeaderChange={onPageHeaderChange}
      />
    );
  if (view === "trust") return <TrustPage client={client} />;
  return <LibraryPanel client={client} />;
}
