import {
  AlertTriangle,
  Cable,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  RotateCw,
  Save,
} from "lucide-react";
import type { ApiClient } from "../../api/client";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import { Button, IconButton, Tooltip } from "../ui";
import {
  FlowInspectorPanel,
  FlowInspectorSection,
} from "../enterprise/FlowInspectorPanel";
import {
  FlowInspectorPortal,
  useFlowWorkspaceTitle,
} from "../enterprise/flowAgentSelection";
import { useEnterpriseStore } from "../enterprise/store";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "../enterprise/pageHeader";
import { ConnectionDetails } from "./ConnectionDetails";
import { ConnectionEditor } from "./ConnectionEditor";
import {
  connectionAccountLabel,
  connectionStatusLabel,
  connectionStatusVariant,
  definitionForConnection,
} from "./model";
import { useConnectionsStore } from "./store";
import "../../styles/connections.css";

const editorFormId = "flow-connection-editor";

export function ConnectionsWorkspacePanel({
  client,
  onPageHeaderChange,
}: {
  client: ApiClient;
  onPageHeaderChange?: EnterprisePageHeaderChange;
}) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useConnectionsStore(client);
  const { snapshot: enterpriseSnapshot } = useEnterpriseStore(client);
  const selected = snapshot.connections.find(
    (connection) => connection.id === snapshot.selectedConnectionId,
  );
  const editorTitle =
    snapshot.editorMode === "edit"
      ? `${t("flow.connection.workspace.editPrefix")} ${selected?.name ?? t("flow.connection.singular")}`
      : t("flow.connection.new");
  const selectedUsage = selected
    ? connectionUsage(enterpriseSnapshot, selected.id)
    : undefined;
  useFlowWorkspaceTitle(snapshot.editorMode ? editorTitle : selected?.name);
  useEnterpriseSubpageHeader(onPageHeaderChange, Boolean(snapshot.editorMode), {
    title:
      snapshot.editorMode === "edit"
        ? t("flow.connection.workspace.subpageEdit")
        : t("flow.connection.workspace.subpageCreate"),
    backLabel: t("flow.connection.workspace.back"),
    onBack: () => store.cancelEdit(),
  });

  if (snapshot.status === "loading" || snapshot.status === "idle") {
    return (
      <div className="connections-page-state" role="status">
        <LoaderCircle
          className="connections-spin"
          aria-hidden="true"
          size={18}
        />
        <strong>{t("flow.connection.workspace.loading")}</strong>
        <span>{t("flow.connection.workspace.loadingDetail")}</span>
      </div>
    );
  }

  if (snapshot.status === "error" && snapshot.connections.length === 0) {
    return (
      <div className="connections-page-state" role="alert">
        <AlertTriangle aria-hidden="true" size={18} />
        <strong>{t("flow.connection.workspace.loadFailed")}</strong>
        <span>{snapshot.error}</span>
        <Button onClick={() => void store.load(true)} variant="primary">
          <RefreshCw aria-hidden="true" size={14} />{" "}
          {t("flow.connection.retry")}
        </Button>
      </div>
    );
  }

  if (snapshot.editorMode) {
    const saving =
      snapshot.busyAction?.startsWith("save:") ||
      snapshot.busyAction === "create";
    return (
      <div className="connections-workspace connections-workspace--editor">
        <div className="connections-editor-stage">
          <Cable aria-hidden="true" size={28} />
          <strong>{editorTitle}</strong>
          <span>{t("flow.connection.workspace.stageDetail")}</span>
        </div>
        <FlowInspectorPortal>
          <FlowInspectorPanel
            actions={
              <>
                <Button
                  disabled={Boolean(saving)}
                  onClick={() => store.cancelEdit()}
                  size="compact"
                  variant="quiet"
                >
                  {t("flow.connection.cancel")}
                </Button>
                <Button
                  disabled={Boolean(saving)}
                  form={editorFormId}
                  size="compact"
                  type="submit"
                  variant="primary"
                >
                  <Save aria-hidden="true" size={14} />
                  {saving
                    ? t("flow.connection.saving")
                    : t("flow.connection.save")}
                </Button>
              </>
            }
            status={t("flow.connection.draft")}
            statusVariant="warning"
            title={
              snapshot.editorMode === "edit"
                ? t("flow.connection.edit")
                : t("flow.connection.new")
            }
          >
            <ConnectionEditor
              formId={editorFormId}
              snapshot={snapshot}
              store={store}
              submitAction="external"
            />
          </FlowInspectorPanel>
        </FlowInspectorPortal>
      </div>
    );
  }

  return (
    <div className="connections-workspace">
      <main className="connections-workspace__detail">
        {selected ? (
          <ConnectionDetails
            connection={selected}
            definition={definitionForConnection(snapshot.definitions, selected)}
            snapshot={snapshot}
            store={store}
            usage={selectedUsage}
            variant="core"
          />
        ) : (
          <div className="connections-empty-state">
            <Cable aria-hidden="true" size={20} />
            <strong>{t("flow.connection.workspace.emptyTitle")}</strong>
            <span>{t("flow.connection.workspace.emptyDetail")}</span>
          </div>
        )}
      </main>

      <FlowInspectorPortal>
        {selected ? (
          <FlowInspectorPanel
            actions={
              <>
                <IconButton
                  aria-label={t("flow.connection.workspace.editAria")}
                  disabled={Boolean(snapshot.busyAction)}
                  onClick={() => store.beginEdit()}
                  size="compact"
                >
                  <Pencil aria-hidden="true" size={14} />
                </IconButton>
                <Tooltip
                  content={t("flow.connection.workspace.testAria")}
                  placement="bottom"
                >
                  {(tooltipProps) => (
                    <IconButton
                      {...tooltipProps}
                      aria-label={t("flow.connection.workspace.testAria")}
                      disabled={
                        Boolean(snapshot.busyAction) || !selected.enabled
                      }
                      onClick={() => void store.test(selected.id)}
                      size="compact"
                    >
                      <RotateCw aria-hidden="true" size={14} />
                    </IconButton>
                  )}
                </Tooltip>
                <Tooltip
                  content={t("flow.connection.workspace.refreshAria")}
                  placement="bottom"
                >
                  {(tooltipProps) => (
                    <IconButton
                      {...tooltipProps}
                      aria-label={t("flow.connection.workspace.refreshAria")}
                      disabled={
                        Boolean(snapshot.busyAction) ||
                        selected.status !== "ready"
                      }
                      onClick={() =>
                        void store.refreshCapabilities(selected.id)
                      }
                      size="compact"
                    >
                      <RefreshCw aria-hidden="true" size={14} />
                    </IconButton>
                  )}
                </Tooltip>
              </>
            }
            status={connectionStatusLabel(selected.status, language)}
            statusVariant={connectionStatusVariant(selected.status)}
            subtitle={connectionAccountLabel(selected, language)}
            title={t("flow.connection.singular")}
          >
            <ConnectionDetails
              connection={selected}
              definition={definitionForConnection(
                snapshot.definitions,
                selected,
              )}
              snapshot={snapshot}
              store={store}
              usage={selectedUsage}
              variant="inspector"
            />
          </FlowInspectorPanel>
        ) : (
          <FlowInspectorPanel
            actions={
              <Button
                onClick={() => store.beginCreate()}
                size="compact"
                variant="primary"
              >
                <Plus aria-hidden="true" size={14} /> {t("flow.connection.new")}
              </Button>
            }
            status={t("flow.connection.empty")}
            title={t("flow.connection.plural")}
          >
            <FlowInspectorSection
              title={t("flow.connection.workspace.configuration")}
            >
              <p>{t("flow.connection.workspace.emptyInspector")}</p>
            </FlowInspectorSection>
          </FlowInspectorPanel>
        )}
      </FlowInspectorPortal>
    </div>
  );
}

function connectionUsage(
  snapshot: ReturnType<typeof useEnterpriseStore>["snapshot"],
  connectionId: string,
) {
  const agentNames = Array.from(
    new Set(
      snapshot.templates
        .filter((view) =>
          view.template.spec.connectionBindings?.some(
            (binding) => binding.connectionId === connectionId,
          ),
        )
        .map((view) => view.template.name),
    ),
  );
  const flowNames = snapshot.flows
    .filter((flow) =>
      Object.values(flow.activeRevision.compiledWorkflow.agentSpecs).some(
        (agent) =>
          agent.connectionBindings.some(
            (binding) => binding.connectionId === connectionId,
          ),
      ),
    )
    .map((flow) => flow.name);
  return { agentNames, flowNames };
}
