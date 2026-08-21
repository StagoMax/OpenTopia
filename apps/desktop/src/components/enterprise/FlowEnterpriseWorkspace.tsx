import type { ApiClient } from "../../api/client";
import type { AppSettings } from "../../types";
import type { FlowPrimaryView } from "../../workspaceNavigation";
import { FlowLibraryPanel } from "../FlowLibraryPanel";
import { HumanTaskInboxPanel } from "../HumanTaskInboxPanel";
import { LibraryPanel } from "../LibraryPanel";
import { WorkflowDeploymentsPanel } from "../workflowDeployments";
import { AgentsPage } from "./AgentsPage";
import { AutomationPage } from "./automation/AutomationPage";
import { OverviewPage } from "./OverviewPage";
import { RunsPage } from "./RunsPage";
import { TrustPage } from "./TrustPage";
import { WorkflowTemplatesPage } from "./WorkflowTemplatesPage";
import "./enterprise.css";

export function FlowEnterpriseWorkspace({
  client,
  settings,
  threadId,
  view,
  workspaceRoot,
  onNavigate,
}: {
  client: ApiClient;
  settings: AppSettings | null;
  threadId: string | null;
  view: Exclude<FlowPrimaryView, "conversation">;
  workspaceRoot: string | null;
  onNavigate(view: Exclude<FlowPrimaryView, "conversation">): void;
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
      />
    );
  if (view === "workflow-templates")
    return <WorkflowTemplatesPage client={client} threadId={threadId} />;
  if (view === "inbox") return <HumanTaskInboxPanel client={client} />;
  if (view === "deployments")
    return (
      <WorkflowDeploymentsPanel activeFlowThreadId={threadId} client={client} />
    );
  if (view === "automation")
    return <AutomationPage client={client} threadId={threadId} />;
  if (view === "runs") return <RunsPage client={client} />;
  if (view === "connections") return <FlowLibraryPanel client={client} />;
  if (view === "trust") return <TrustPage client={client} />;
  return <LibraryPanel client={client} />;
}
