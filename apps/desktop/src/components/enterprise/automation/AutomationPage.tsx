import {
  AlertTriangle,
  CheckCircle2,
  FlaskConical,
  GitCompareArrows,
  LoaderCircle,
  Plus,
  RadioTower,
  RefreshCw,
  RotateCcw,
  Send,
  ShieldCheck,
} from "lucide-react";
import { useMemo, useState, type FormEvent } from "react";
import type { ApiClient } from "../../../api/client";
import type { WorkflowRelease, WorkflowTrigger } from "../../../types";
import {
  Badge,
  Button,
  IconButton,
  Panel,
  SelectField,
  TextField,
} from "../../ui";
import {
  useEnterpriseSubpageHeader,
  type EnterprisePageHeaderChange,
} from "../pageHeader";
import { useWorkflowAutomationStore } from "./store";
import "./automation.css";

export function AutomationPage({
  client,
  onPageHeaderChange,
  threadId,
}: {
  client: ApiClient;
  onPageHeaderChange?: EnterprisePageHeaderChange;
  threadId: string | null;
}) {
  const { snapshot, store } = useWorkflowAutomationStore(client);
  useEnterpriseSubpageHeader(onPageHeaderChange, snapshot.createOpen, {
    title: "Automation / 创建发布通道",
    backLabel: "返回 Automation",
    onBack: () => store.setCreateOpen(false),
  });
  const selected =
    snapshot.releases.find(
      (release) => release.id === snapshot.selectedReleaseId,
    ) ?? null;
  const failedDeliveries = snapshot.receipts.filter(
    (receipt) => receipt.status === "failed",
  ).length;

  if (snapshot.status === "loading" || snapshot.status === "idle") {
    return (
      <div className="automation-state" role="status">
        <LoaderCircle
          aria-hidden="true"
          className="automation-spin"
          size={18}
        />
        <strong>加载 Automation Control Plane</strong>
        <span>读取 Release、TriggerInvocation 与 DeliveryReceipt…</span>
      </div>
    );
  }

  if (snapshot.createOpen) {
    return (
      <div className="enterprise-page automation-page automation-page--create">
        {snapshot.error ? (
          <div className="automation-feedback is-error" role="alert">
            <span>{snapshot.error}</span>
            <Button
              onClick={() => store.clearFeedback()}
              size="compact"
              variant="quiet"
            >
              关闭
            </Button>
          </div>
        ) : null}
        <CreateReleaseForm
          deployments={snapshot.deployments}
          onCreate={(input) => store.create(input)}
          threadId={threadId}
        />
      </div>
    );
  }

  return (
    <div className="enterprise-page automation-page">
      <Panel
        title="Automation & delivery / 自动化与投递"
        actions={
          <>
            <Button
              onClick={() => store.setCreateOpen(true)}
              size="compact"
              variant="primary"
            >
              <Plus aria-hidden="true" size={14} />
              New Release
            </Button>
            <IconButton
              aria-label="刷新自动化控制面"
              disabled={Boolean(snapshot.busyAction)}
              onClick={() => void store.load(true)}
              size="compact"
            >
              <RefreshCw aria-hidden="true" size={14} />
            </IconButton>
          </>
        }
      >
        <p className="enterprise-page__lede">
          稳定 Release Channel 接收 Webhook、Schedule 与 Event；Deployment
          Snapshot 冻结输出，DeliveryReceipt 提供审计、重试和人工恢复。
        </p>
        {snapshot.error || snapshot.notice ? (
          <div
            className={`automation-feedback${snapshot.error ? " is-error" : ""}`}
            role={snapshot.error ? "alert" : "status"}
          >
            <span>{snapshot.error ?? snapshot.notice}</span>
            <Button
              onClick={() => store.clearFeedback()}
              size="compact"
              variant="quiet"
            >
              关闭
            </Button>
          </div>
        ) : null}
        <div className="automation-metrics" aria-label="自动化运行指标">
          <Metric
            icon={RadioTower}
            label="Active releases"
            value={
              snapshot.releases.filter((item) => item.status === "active")
                .length
            }
          />
          <Metric
            icon={Send}
            label="Invocations"
            value={snapshot.invocations.length}
          />
          <Metric
            icon={CheckCircle2}
            label="Delivered"
            value={
              snapshot.receipts.filter((item) => item.status === "delivered")
                .length
            }
          />
          <Metric
            danger={failedDeliveries > 0}
            icon={AlertTriangle}
            label="Delivery failures"
            value={failedDeliveries}
          />
        </div>
      </Panel>

      <div className="automation-layout">
        <Panel
          title={`Release channels / 发布通道 · ${snapshot.releases.length}`}
        >
          <ol className="automation-release-list">
            {snapshot.releases.map((release) => (
              <li key={release.id}>
                <button
                  aria-current={
                    release.id === snapshot.selectedReleaseId
                      ? "true"
                      : undefined
                  }
                  onClick={() => store.select(release.id)}
                  type="button"
                >
                  <RadioTower aria-hidden="true" size={15} />
                  <span>
                    <strong>{release.releaseKey}</strong>
                    <small>
                      {triggerLabel(release.trigger)} · {release.environment}
                    </small>
                  </span>
                  <Badge
                    variant={
                      release.status === "active" ? "success" : "neutral"
                    }
                  >
                    {release.status}
                  </Badge>
                </button>
              </li>
            ))}
            {snapshot.releases.length === 0 ? (
              <li className="enterprise-list__empty">
                尚无 Release Channel。选择一个 Active Deployment 创建外部入口。
              </li>
            ) : null}
          </ol>
        </Panel>
        {selected ? (
          <ReleaseDetails
            key={selected.id}
            release={selected}
            snapshot={snapshot}
            store={store}
          />
        ) : (
          <Panel title="Release detail / 发布详情">
            <p className="automation-muted">
              选择或创建 Release Channel 后查看 Canary、Delivery 与 Evaluation。
            </p>
          </Panel>
        )}
      </div>
    </div>
  );
}

function Metric({
  danger = false,
  icon: Icon,
  label,
  value,
}: {
  danger?: boolean;
  icon: typeof RadioTower;
  label: string;
  value: number;
}) {
  return (
    <span className={`automation-metric${danger ? " is-danger" : ""}`}>
      <Icon aria-hidden="true" size={15} />
      <strong>{value}</strong>
      <small>{label}</small>
    </span>
  );
}

function CreateReleaseForm({
  deployments,
  onCreate,
  threadId,
}: {
  deployments: readonly import("../../../types").WorkflowDeployment[];
  onCreate(input: {
    releaseKey: string;
    environment: string;
    threadId: string;
    deploymentId: string;
    trigger: WorkflowTrigger;
    createdBy: string;
  }): Promise<boolean>;
  threadId: string | null;
}) {
  const active = deployments.filter((item) => item.status === "active");
  const [deploymentId, setDeploymentId] = useState(active[0]?.id ?? "");
  const deployment = active.find((item) => item.id === deploymentId);
  const [releaseKey, setReleaseKey] = useState("");
  const [kind, setKind] =
    useState<Exclude<WorkflowTrigger["kind"], "manual">>("webhook");
  const [tokenRef, setTokenRef] = useState("env:WORKFLOW_TRIGGER_TOKEN");
  const [intervalSeconds, setIntervalSeconds] = useState("3600");
  const [nextFireAt, setNextFireAt] = useState(
    new Date(Date.now() + 60_000).toISOString(),
  );
  const [eventSource, setEventSource] = useState("crm");
  const [eventType, setEventType] = useState("record.updated");
  const [createdBy, setCreatedBy] = useState("local-user");

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!threadId || !deployment || !releaseKey.trim()) return;
    const triggerId = crypto.randomUUID();
    const trigger: WorkflowTrigger =
      kind === "webhook"
        ? { kind, triggerId, tokenRef: tokenRef.trim() }
        : kind === "schedule"
          ? {
              kind,
              triggerId,
              intervalSeconds: Number(intervalSeconds),
              nextFireAt: nextFireAt.trim(),
            }
          : {
              kind,
              triggerId,
              source: eventSource.trim(),
              eventType: eventType.trim(),
            };
    await onCreate({
      releaseKey: releaseKey.trim(),
      environment: deployment.environment,
      threadId,
      deploymentId: deployment.id,
      trigger,
      createdBy: createdBy.trim(),
    });
  }

  return (
    <Panel title="Release configuration / 发布通道配置">
      <form
        className="automation-create-form"
        onSubmit={(event) => void submit(event)}
      >
        <div className="automation-form-grid">
          <TextField
            label="Release key / 稳定标识"
            onChange={(event) => setReleaseKey(event.target.value)}
            required
            value={releaseKey}
          />
          <SelectField
            label="Primary Deployment / 主部署"
            onChange={setDeploymentId}
            options={active.map((item) => ({
              value: item.id,
              label: `${item.name} · ${item.environment}`,
            }))}
            value={deploymentId}
          />
          <SelectField
            label="Trigger / 触发器"
            onChange={(value) => setKind(value as typeof kind)}
            options={[
              { value: "webhook", label: "Webhook / 外部接口" },
              { value: "schedule", label: "Schedule / 定时" },
              {
                value: "event_subscription",
                label: "Event Subscription / 事件订阅",
              },
            ]}
            value={kind}
          />
          {kind === "webhook" ? (
            <TextField
              hint="只保存 env: 引用，不保存 Token"
              label="Token ref / Token 引用"
              onChange={(event) => setTokenRef(event.target.value)}
              required
              value={tokenRef}
            />
          ) : null}
          {kind === "schedule" ? (
            <>
              <TextField
                label="Interval seconds / 间隔秒数"
                onChange={(event) => setIntervalSeconds(event.target.value)}
                required
                value={intervalSeconds}
              />
              <TextField
                label="Next fire at / 下次触发（RFC3339）"
                onChange={(event) => setNextFireAt(event.target.value)}
                required
                value={nextFireAt}
              />
            </>
          ) : null}
          {kind === "event_subscription" ? (
            <>
              <TextField
                label="Event source / 事件源"
                onChange={(event) => setEventSource(event.target.value)}
                required
                value={eventSource}
              />
              <TextField
                label="Event type / 事件类型"
                onChange={(event) => setEventType(event.target.value)}
                required
                value={eventType}
              />
            </>
          ) : null}
          <TextField
            label="Created by / 创建人"
            onChange={(event) => setCreatedBy(event.target.value)}
            required
            value={createdBy}
          />
        </div>
        {!threadId ? (
          <p className="automation-warning">
            <AlertTriangle aria-hidden="true" size={14} />
            先打开 Flow 任务；外部 Run 会归属到该任务。
          </p>
        ) : null}
        <Button
          disabled={!threadId || !deployment || !releaseKey.trim()}
          type="submit"
          variant="primary"
        >
          <RadioTower aria-hidden="true" size={14} />
          Activate Release / 激活
        </Button>
      </form>
    </Panel>
  );
}

function ReleaseDetails({
  release,
  snapshot,
  store,
}: {
  release: WorkflowRelease;
  snapshot: ReturnType<typeof useWorkflowAutomationStore>["snapshot"];
  store: ReturnType<typeof useWorkflowAutomationStore>["store"];
}) {
  const canaryOptions = snapshot.deployments.filter(
    (item) =>
      item.status === "active" &&
      item.environment === release.environment &&
      item.id !== release.primaryDeploymentId,
  );
  const [canaryId, setCanaryId] = useState(canaryOptions[0]?.id ?? "");
  const [percent, setPercent] = useState("10");
  const receipts = snapshot.receipts
    .filter(
      (item) =>
        item.deploymentId === release.primaryDeploymentId ||
        item.deploymentId === release.canaryDeploymentId,
    )
    .slice(0, 20);
  const busy = Boolean(snapshot.busyAction);
  return (
    <div className="automation-detail">
      <Panel title={`${release.releaseKey} / Release`}>
        <dl className="automation-definition-list">
          <div>
            <dt>Trigger / 触发器</dt>
            <dd>{triggerLabel(release.trigger)}</dd>
          </div>
          <div>
            <dt>Primary Deployment</dt>
            <dd>
              <code>{shortId(release.primaryDeploymentId)}</code>
            </dd>
          </div>
          <div>
            <dt>Canary</dt>
            <dd>
              {release.canaryDeploymentId
                ? `${release.canaryPercent}% · ${shortId(release.canaryDeploymentId)}`
                : "未启用"}
            </dd>
          </div>
          <div>
            <dt>Revision</dt>
            <dd>{release.revision}</dd>
          </div>
        </dl>
        <div className="automation-actions">
          {release.status === "active" && canaryOptions.length > 0 ? (
            <>
              <SelectField
                label="Canary deployment"
                onChange={setCanaryId}
                options={canaryOptions.map((item) => ({
                  value: item.id,
                  label: item.name,
                }))}
                value={canaryId}
              />
              <TextField
                label="Traffic %"
                onChange={(event) => setPercent(event.target.value)}
                value={percent}
              />
              <Button
                disabled={busy || !canaryId}
                onClick={() =>
                  void store.setCanary(release, canaryId, Number(percent))
                }
                size="compact"
                variant="quiet"
              >
                <GitCompareArrows aria-hidden="true" size={14} />
                Set Canary
              </Button>
            </>
          ) : null}
          {release.canaryDeploymentId ? (
            <Button
              disabled={busy}
              onClick={() => void store.promote(release)}
              size="compact"
              variant="primary"
            >
              <ShieldCheck aria-hidden="true" size={14} />
              Promote
            </Button>
          ) : null}
          {release.previousPrimaryDeploymentId ? (
            <Button
              disabled={busy}
              onClick={() => void store.rollback(release)}
              size="compact"
              variant="quiet"
            >
              <RotateCcw aria-hidden="true" size={14} />
              Rollback
            </Button>
          ) : null}
          {release.status === "active" ? (
            <Button
              disabled={busy}
              onClick={() => void store.disable(release)}
              size="compact"
              variant="danger"
            >
              Disable
            </Button>
          ) : null}
        </div>
      </Panel>
      <Panel title="Evaluation / 评估">
        <div className="automation-summary">
          {snapshot.summaryLoading ? (
            <LoaderCircle
              aria-hidden="true"
              className="automation-spin"
              size={16}
            />
          ) : (
            <>
              <span>
                <strong>{snapshot.summary?.totalRuns ?? 0}</strong>
                <small>Runs</small>
              </span>
              <span>
                <strong>{formatRate(snapshot.summary?.passRate)}</strong>
                <small>Pass rate</small>
              </span>
              <span>
                <strong>{formatScore(snapshot.summary?.averageScore)}</strong>
                <small>Avg score</small>
              </span>
              <span>
                <strong>{snapshot.summary?.failureClusters.length ?? 0}</strong>
                <small>Failure clusters</small>
              </span>
            </>
          )}
        </div>
        {snapshot.summary?.failureClusters.map((cluster) => (
          <p className="automation-cluster" key={cluster.key}>
            <FlaskConical aria-hidden="true" size={14} />
            <span>
              <strong>{cluster.key}</strong>
              <small>{cluster.sample}</small>
            </span>
            <Badge variant="danger">{cluster.count}</Badge>
          </p>
        ))}
      </Panel>
      <Panel title={`Delivery receipts / 投递收据 · ${receipts.length}`}>
        <ol className="automation-receipts">
          {receipts.map((receipt) => (
            <li key={receipt.id}>
              <Send aria-hidden="true" size={14} />
              <span>
                <strong>{receipt.outputKind}</strong>
                <small>
                  {shortId(receipt.runId)} · attempt {receipt.attempt}
                </small>
                {receipt.error ? (
                  <small className="is-error">{receipt.error}</small>
                ) : null}
              </span>
              <Badge
                variant={
                  receipt.status === "delivered"
                    ? "success"
                    : receipt.status === "failed"
                      ? "danger"
                      : "warning"
                }
              >
                {receipt.status}
              </Badge>
              {receipt.status === "failed" ? (
                <Button
                  disabled={busy}
                  onClick={() => void store.retry(receipt)}
                  size="compact"
                  variant="quiet"
                >
                  <RotateCcw aria-hidden="true" size={13} />
                  Retry
                </Button>
              ) : null}
            </li>
          ))}
          {receipts.length === 0 ? (
            <li className="enterprise-list__empty">暂无投递记录。</li>
          ) : null}
        </ol>
      </Panel>
    </div>
  );
}

function triggerLabel(trigger: WorkflowTrigger): string {
  if (trigger.kind === "event_subscription")
    return `Event · ${trigger.source}/${trigger.eventType}`;
  if (trigger.kind === "schedule")
    return `Schedule · ${trigger.intervalSeconds}s`;
  if (trigger.kind === "webhook")
    return `Webhook · ${shortId(trigger.triggerId)}`;
  return "Manual";
}

function shortId(value: string): string {
  return value.slice(0, 8);
}
function formatRate(value?: number | null): string {
  return value == null ? "—" : `${Math.round(value * 100)}%`;
}
function formatScore(value?: number | null): string {
  return value == null ? "—" : value.toFixed(2);
}
