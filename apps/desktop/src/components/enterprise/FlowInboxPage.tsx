import { CheckCircle2, Clock3, Play, RefreshCw, UserCheck } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ApiClient } from "../../api/client";
import {
  humanTaskActionPresentation,
  humanTaskInputRequest,
  humanTaskTypeLabel,
  orderedHumanTaskActions,
} from "../../humanTasks";
import type {
  FlowCase,
  HumanTask,
  HumanTaskAction,
  UserInputResponse,
} from "../../types";
import { PlanChoiceCard } from "../PlanChoiceCard";
import { Badge, Button, IconButton, TextField } from "../ui";
import { FlowInspectorPanel, FlowInspectorSection } from "./FlowInspectorPanel";
import {
  FlowInspectorPortal,
  useFlowWorkspaceSelection,
  useFlowWorkspaceTitle,
} from "./flowAgentSelection";
import { workflowTriggerLabel } from "./enterpriseSidebarPresentation";
import { useEnterpriseStore } from "./store";
import { StructuredPayload } from "./StructuredPayload";
import { useApplicationLanguage } from "../../ApplicationLanguageProvider";
import {
  interfaceMessage,
  type ApplicationLanguage,
} from "../../applicationLanguage";

type InboxItem =
  | { id: string; kind: "task"; task: HumanTask }
  | { id: string; kind: "case"; flowCase: FlowCase };

export function FlowInboxPage({ client }: { client: ApiClient }) {
  const { language, t } = useApplicationLanguage();
  const { snapshot, store } = useEnterpriseStore(client);
  const workspace = useFlowWorkspaceSelection();
  const [busy, setBusy] = useState<string | null>(null);
  const [note, setNote] = useState("");
  const [response, setResponse] = useState("");
  const [error, setError] = useState<string | null>(null);

  const items = useMemo<InboxItem[]>(() => {
    const tasks = snapshot.tasks.map((task) => ({
      id: `task:${task.id}`,
      kind: "task" as const,
      task,
    }));
    const cases = snapshot.cases
      .filter((item) => item.status === "accepted" && !item.flowRunId)
      .map((flowCase) => ({
        id: `case:${flowCase.id}`,
        kind: "case" as const,
        flowCase,
      }));
    return [...tasks, ...cases].sort(
      (left, right) => inboxTime(left) - inboxTime(right),
    );
  }, [snapshot.cases, snapshot.tasks]);

  const selected =
    items.find((item) => item.id === workspace?.selectedInboxItemId) ??
    items[0] ??
    null;

  useEffect(() => {
    if (selected && selected.id !== workspace?.selectedInboxItemId) {
      workspace?.setSelectedInboxItemId(selected.id);
    }
  }, [selected, workspace]);

  useEffect(() => {
    setNote("");
    setResponse("");
    setError(null);
  }, [selected?.id]);

  const selectedFlow =
    selected?.kind === "case"
      ? (snapshot.flows.find(
          (flow) => flow.flowId === selected.flowCase.flowId,
        ) ?? null)
      : null;
  const title =
    selected?.kind === "task"
      ? selected.task.title
      : selected?.kind === "case"
        ? (selectedFlow?.name ?? selected.flowCase.flowId)
        : t("flow.inbox.workspaceTitle");
  useFlowWorkspaceTitle(title);

  async function runAction(
    action: HumanTaskAction,
    explicitResponse?: unknown,
  ) {
    if (selected?.kind !== "task" || busy) return;
    setBusy(action);
    setError(null);
    try {
      const parsedResponse =
        explicitResponse ??
        (response.trim() ? (JSON.parse(response) as unknown) : undefined);
      await client.resolveHumanTask(selected.task.id, {
        expectedRevision: selected.task.revision,
        action,
        note: note.trim() || undefined,
        idempotencyKey: crypto.randomUUID(),
        response: parsedResponse,
      });
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause, language));
    } finally {
      setBusy(null);
    }
  }

  async function startCase() {
    if (selected?.kind !== "case" || busy) return;
    setBusy("start");
    setError(null);
    try {
      await client.startPendingFlowCase(selected.flowCase.id);
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause, language));
    } finally {
      setBusy(null);
    }
  }

  async function claimTask() {
    if (selected?.kind !== "task" || busy) return;
    setBusy("claim");
    setError(null);
    try {
      await client.claimHumanTask(selected.task.id, selected.task.revision);
      await store.load(true);
    } catch (cause) {
      setError(readableError(cause, language));
    } finally {
      setBusy(null);
    }
  }

  if (!selected) {
    return (
      <div className="enterprise-page enterprise-page--empty">
        <CheckCircle2 aria-hidden="true" size={28} />
        <strong>{t("flow.inbox.emptyTitle")}</strong>
        <span>{t("flow.inbox.emptyDescription")}</span>
        <FlowInspectorPortal>
          <FlowInspectorPanel
            actions={
              <IconButton
                aria-label={t("flow.inbox.refresh")}
                onClick={() => void store.load(true)}
                size="compact"
              >
                <RefreshCw aria-hidden="true" size={14} />
              </IconButton>
            }
            status="clear"
            statusVariant="success"
            title={t("flow.nav.inbox")}
          >
            <p className="enterprise-page__lede">
              {t("flow.inbox.emptyInspector")}
            </p>
          </FlowInspectorPanel>
        </FlowInspectorPortal>
      </div>
    );
  }

  const actions =
    selected.kind === "task" ? orderedHumanTaskActions(selected.task) : [];
  const inputRequest =
    selected.kind === "task" ? humanTaskInputRequest(selected.task) : null;
  const requiresObservation =
    selected.kind === "task" &&
    ["reconciliation", "reconnect"].includes(selected.task.taskType);
  const payload =
    selected.kind === "task" ? selected.task.payload : selected.flowCase.input;
  const payloadSchema =
    selected.kind === "case"
      ? selected.flowCase.flowRevision.compiledWorkflow.inputSchema
      : undefined;

  return (
    <div className="enterprise-page enterprise-inbox-page enterprise-core-detail">
      <section className="enterprise-core-detail__summary">
        <span className="enterprise-core-detail__icon" aria-hidden="true">
          {selected.kind === "task" ? (
            <UserCheck size={20} />
          ) : (
            <Clock3 size={20} />
          )}
        </span>
        <div>
          <small>
            {selected.kind === "task"
              ? humanTaskTypeLabel(selected.task.taskType, language)
              : t("flow.inbox.pendingEvent")}
          </small>
          <h2>{title}</h2>
          <p>
            {selected.kind === "task"
              ? selected.task.description
              : t("flow.inbox.pendingEventDescription")}
          </p>
        </div>
      </section>
      <section className="enterprise-core-detail__payload">
        <header>
          <h3>{t("flow.inbox.confirmationInfo")}</h3>
          <Badge variant="neutral">
            {selected.kind === "task"
              ? humanTaskSourceLabel(selected.task.sourceKind, language)
              : workflowTriggerLabel(
                  selected.flowCase.flowRevision.trigger,
                  language,
                )}
          </Badge>
        </header>
        <StructuredPayload
          emptyLabel={t("flow.inbox.noAdditionalInfo")}
          schema={payloadSchema}
          value={payload}
        />
      </section>

      <FlowInspectorPortal>
        <FlowInspectorPanel
          actions={
            selected.kind === "case" ? (
              <Button
                disabled={Boolean(busy)}
                onClick={() => void startCase()}
                size="compact"
                variant="primary"
              >
                <Play aria-hidden="true" size={14} />
                {busy === "start"
                  ? t("flow.inbox.starting")
                  : t("flow.inbox.approveAndRun")}
              </Button>
            ) : inputRequest ? null : (
              actions.map((action) => {
                const presentation = humanTaskActionPresentation(
                  action,
                  language,
                );
                return (
                  <Button
                    disabled={
                      Boolean(busy) ||
                      (requiresObservation &&
                        ["resume", "reconnect", "acknowledge"].includes(
                          action,
                        ) &&
                        !note.trim())
                    }
                    key={action}
                    onClick={() => void runAction(action)}
                    size="compact"
                    variant={presentation.variant}
                  >
                    {busy === action
                      ? presentation.pendingLabel
                      : presentation.label}
                  </Button>
                );
              })
            )
          }
          status={t("flow.inbox.pending")}
          statusVariant="warning"
          subtitle={
            selected.kind === "task"
              ? humanTaskTypeLabel(selected.task.taskType, language)
              : selectedFlow?.name
          }
          title={t("flow.inbox.item")}
        >
          {snapshot.error || error ? (
            <p className="enterprise-page__message is-error" role="alert">
              {snapshot.error ?? error}
            </p>
          ) : null}
          <FlowInspectorSection title={t("flow.inbox.details")}>
            {selected.kind === "task" ? (
              <>
                <dl className="flow-inspector-facts">
                  <div>
                    <dt>{t("flow.inbox.owner")}</dt>
                    <dd>
                      {selected.task.assignedTo ?? t("flow.inbox.unassigned")}
                    </dd>
                  </div>
                  <div>
                    <dt>{t("flow.inbox.claimStatus")}</dt>
                    <dd>
                      {selected.task.claimedBy
                        ? `${selected.task.claimedBy} ${t("flow.inbox.claimed")}`
                        : t("flow.inbox.unclaimed")}
                    </dd>
                  </div>
                  <div>
                    <dt>
                      {selected.task.dueAt
                        ? t("flow.inbox.dueAt")
                        : t("flow.inbox.receivedAt")}
                    </dt>
                    <dd>
                      {formatTime(
                        selected.task.dueAt ?? selected.task.createdAt,
                        language,
                      )}
                    </dd>
                  </div>
                </dl>
                {!selected.task.claimedBy ? (
                  <Button
                    disabled={Boolean(busy)}
                    onClick={() => void claimTask()}
                    size="compact"
                    variant="secondary"
                  >
                    {busy === "claim"
                      ? t("flow.inbox.claiming")
                      : t("flow.inbox.claim")}
                  </Button>
                ) : null}
              </>
            ) : (
              <dl className="flow-inspector-facts">
                <div>
                  <dt>{t("flow.inbox.flow")}</dt>
                  <dd>{selectedFlow?.name ?? selected.flowCase.flowId}</dd>
                </div>
                <div>
                  <dt>{t("flow.inbox.trigger")}</dt>
                  <dd>
                    {workflowTriggerLabel(
                      selected.flowCase.flowRevision.trigger,
                      language,
                    )}
                  </dd>
                </div>
                <div>
                  <dt>{t("flow.inbox.receivedAt")}</dt>
                  <dd>{formatTime(selected.flowCase.createdAt, language)}</dd>
                </div>
              </dl>
            )}
          </FlowInspectorSection>
          {selected.kind === "task" ? (
            <FlowInspectorSection title={t("flow.inbox.instructions")}>
              {inputRequest ? (
                <PlanChoiceCard
                  error={null}
                  isSubmitting={Boolean(busy)}
                  onCancel={() => void runAction("cancel")}
                  onSkip={() =>
                    void runAction("submit", {
                      answers: [],
                      skipped: true,
                    } satisfies UserInputResponse)
                  }
                  onSubmit={(value) => void runAction("submit", value)}
                  request={inputRequest}
                />
              ) : (
                <TextField
                  hint={
                    requiresObservation
                      ? t("flow.inbox.observationHint")
                      : undefined
                  }
                  label={
                    requiresObservation
                      ? t("flow.inbox.observation")
                      : t("flow.inbox.note")
                  }
                  onChange={(event) => setNote(event.currentTarget.value)}
                  value={note}
                />
              )}
              {!inputRequest && selected.task.actionSchema ? (
                <details className="enterprise-technical-details">
                  <summary>{t("flow.inbox.advancedResponse")}</summary>
                  <label className="flow-workspace-inspector__textarea">
                    <span>{t("flow.inbox.structuredResponse")}</span>
                    <textarea
                      onChange={(event) =>
                        setResponse(event.currentTarget.value)
                      }
                      placeholder="{}"
                      rows={6}
                      value={response}
                    />
                  </label>
                </details>
              ) : null}
            </FlowInspectorSection>
          ) : null}
          <FlowInspectorSection title={t("flow.inbox.technical")}>
            <details className="enterprise-technical-details">
              <summary>{t("flow.inbox.showTechnicalIds")}</summary>
              <dl className="flow-inspector-facts">
                <div>
                  <dt>{t("flow.inbox.itemId")}</dt>
                  <dd>
                    <code>{selected.id.replace(/^(task|case):/, "")}</code>
                  </dd>
                </div>
                {selected.kind === "case" ? (
                  <>
                    <div>
                      <dt>{t("flow.inbox.revisionId")}</dt>
                      <dd>
                        <code>{selected.flowCase.flowRevisionId}</code>
                      </dd>
                    </div>
                    <div>
                      <dt>{t("flow.inbox.triggerId")}</dt>
                      <dd>
                        <code>{selected.flowCase.triggerId}</code>
                      </dd>
                    </div>
                  </>
                ) : null}
              </dl>
            </details>
          </FlowInspectorSection>
        </FlowInspectorPanel>
      </FlowInspectorPortal>
    </div>
  );
}

function readableError(error: unknown, language: ApplicationLanguage): string {
  if (error instanceof SyntaxError) {
    return interfaceMessage(language, "flow.inbox.invalidJson");
  }
  return error instanceof Error ? error.message : String(error);
}

function formatTime(value: string, language: ApplicationLanguage): string {
  return new Date(value).toLocaleString(language);
}

function humanTaskSourceLabel(
  source: HumanTask["sourceKind"],
  language: ApplicationLanguage,
): string {
  return interfaceMessage(
    language,
    source === "flow_run"
      ? "flow.inbox.sourceRun"
      : "flow.inbox.sourceExternal",
  );
}

function inboxTime(item: InboxItem): number {
  const value =
    item.kind === "task"
      ? (item.task.dueAt ?? item.task.createdAt)
      : item.flowCase.createdAt;
  const timestamp = new Date(value).getTime();
  return Number.isFinite(timestamp) ? timestamp : Number.MAX_SAFE_INTEGER;
}
