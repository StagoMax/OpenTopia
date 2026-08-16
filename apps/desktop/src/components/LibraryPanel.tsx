import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type FormEvent,
} from "react";
import {
  BookOpen,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  Clock3,
  Database,
  FilePlus2,
  FileText,
  GitBranch,
  Layers3,
  Network,
  RefreshCw,
  Search,
  ShieldCheck,
  Upload,
  Waypoints,
  XCircle,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import {
  coverageByNeed,
  parseSagMetadata,
  sagErrorMessage,
} from "../sagLibrary";
import { ensureLibraryProviderService } from "../platform";
import type {
  GraphRagIngestionResult,
  GraphRagSearchResponse,
  LibraryIngestionResult,
  LibraryProviderDescriptor,
  LibraryProviderId,
  LibraryProviderServiceRuntimeStatus,
  LibraryProviderStatus,
  LibrarySearchResponse,
  LibrarySource,
  LibrarySourcePage,
  SagIngestionResult,
  SagLibraryStatus,
  SagSearchResponse,
  SagSource,
} from "../types";
import {
  Badge,
  Button,
  NumberField,
  Panel,
  SegmentedControl,
  Switch,
  TextField,
} from "./ui";
import "../styles/sag-library-panel.css";

type LibrarySection = "search" | "sources" | "ingest";
type GraphRetrievalMode = "auto" | "hybrid" | "graph";

const providerStorageKey = "opentopia.library.provider";
const fallbackProviders: LibraryProviderDescriptor[] = [
  {
    id: "sag",
    name: "SAG",
    title: "SAG 记忆检索",
    description: "事件、实体与时序记忆的多路检索",
    capabilities: {
      graphPaths: false,
      temporalMemory: true,
      incrementalUpload: true,
      llmPlanning: true,
    },
  },
  {
    id: "graph-rag",
    name: "Graph RAG",
    title: "Graph RAG 图谱检索",
    description: "混合召回、关系扩展与路径解释",
    capabilities: {
      graphPaths: true,
      temporalMemory: false,
      incrementalUpload: true,
      llmPlanning: false,
    },
  },
];

function initialProvider(): LibraryProviderId {
  const saved = localStorage.getItem(providerStorageKey);
  return saved === "graph-rag" ? "graph-rag" : "sag";
}

export function LibraryPanel({ client }: { client: ApiClient | null }) {
  const [provider, setProviderState] =
    useState<LibraryProviderId>(initialProvider);
  const [providers, setProviders] = useState(fallbackProviders);
  const [section, setSection] = useState<LibrarySection>("search");
  const [status, setStatus] = useState<LibraryProviderStatus | null>(null);
  const [sourceRevision, setSourceRevision] = useState(0);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [runtime, setRuntime] =
    useState<LibraryProviderServiceRuntimeStatus | null>(null);

  const setProvider = useCallback((next: LibraryProviderId) => {
    localStorage.setItem(providerStorageKey, next);
    setProviderState(next);
    setSection("search");
  }, []);

  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      if (!client) {
        setStatus(null);
        setLoadError("OpenTopia 服务尚未连接。请稍后重试。");
        setLoading(false);
        return;
      }
      setLoading(true);
      setStatus(null);
      setLoadError(null);
      try {
        const runtimeStatus = await ensureLibraryProviderService(provider);
        if (signal?.aborted) return;
        setRuntime(runtimeStatus);
        if (runtimeStatus?.state === "unavailable") {
          throw new Error(
            runtimeStatus.message ||
              `${providerName(provider)} 本地服务尚未就绪。`,
          );
        }
        const nextStatus = await client.getLibraryProviderStatus(
          provider,
          signal,
        );
        if (signal?.aborted) return;
        setStatus(nextStatus);
      } catch (cause) {
        if (signal?.aborted) return;
        setLoadError(sagErrorMessage(cause));
      } finally {
        if (!signal?.aborted) setLoading(false);
      }
    },
    [client, provider],
  );

  useEffect(() => {
    if (!client) return;
    const controller = new AbortController();
    void client
      .listLibraryProviders(controller.signal)
      .then((items) => {
        if (!controller.signal.aborted && items.length) setProviders(items);
      })
      .catch(() => undefined);
    return () => controller.abort();
  }, [client]);

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  const activeDescriptor =
    providers.find((item) => item.id === provider) ?? fallbackProviders[0];

  return (
    <section className="sag-library" aria-labelledby="library-title">
      <header className="sag-library__header">
        <span className="sag-library__header-icon">
          <Network aria-hidden="true" size={18} />
        </span>
        <span className="sag-library__heading">
          <small>Library / 资料库</small>
          <strong id="library-title">知识与记忆检索</strong>
          <span>选择检索后端，构造并审阅独立的 Context Pack</span>
        </span>
        <span className="sag-library__header-actions">
          <Badge variant={status ? "success" : loading ? "info" : "warning"}>
            {status ? "服务已连接" : loading ? "正在启动" : "需要处理"}
          </Badge>
          <Button
            disabled={loading}
            onClick={() => {
              setSourceRevision((value) => value + 1);
              void refresh();
            }}
            size="compact"
            variant="quiet"
          >
            <RefreshCw aria-hidden="true" size={14} /> 刷新
          </Button>
        </span>
      </header>

      <ProviderPicker
        onChange={setProvider}
        providers={providers}
        value={provider}
      />

      <section className="sag-library__safety" aria-label="集成边界">
        <ShieldCheck aria-hidden="true" size={16} />
        <span>
          <strong>审阅模式 · {activeDescriptor.name}</strong>
          <small>
            当前只构造待审 Context Pack，不写入提示词，也不改变 Agent Loop。
          </small>
        </span>
        <Badge variant="neutral">Prompt 注入：关闭</Badge>
      </section>

      {loadError ? (
        <section className="sag-library__error" role="alert">
          <XCircle aria-hidden="true" size={16} />
          <span>
            <strong>无法连接 {providerName(provider)}</strong>
            <small>{loadError}</small>
            {runtime?.endpoint ? <code>{runtime.endpoint}</code> : null}
          </span>
          <Button
            disabled={loading}
            onClick={() => {
              setSourceRevision((value) => value + 1);
              void refresh();
            }}
            size="compact"
            variant="secondary"
          >
            <RefreshCw aria-hidden="true" size={14} />
            {runtime?.canStart ? "重新启动" : "重新检测"}
          </Button>
        </section>
      ) : null}

      {status ? <LibraryStatusStrip value={status} /> : null}

      <SegmentedControl<LibrarySection>
        className="sag-library__sections"
        label={`${providerName(provider)} 资料库功能`}
        onChange={setSection}
        options={[
          { value: "search", label: "检索审阅" },
          { value: "sources", label: "资料来源" },
          { value: "ingest", label: "增量导入" },
        ]}
        value={section}
      />

      {section === "search" ? (
        <LibrarySearchPanel
          key={`search:${provider}`}
          client={client}
          disabled={!status}
          provider={provider}
        />
      ) : section === "sources" ? (
        <LibrarySourcesPanel
          client={client}
          disabled={!status}
          provider={provider}
          revision={sourceRevision}
        />
      ) : (
        <LibraryIngestionPanel
          key={`ingest:${provider}`}
          client={client}
          disabled={!status}
          provider={provider}
          onImported={async () => {
            await refresh();
            setSourceRevision((value) => value + 1);
            setSection("sources");
          }}
        />
      )}
    </section>
  );
}

function ProviderPicker({
  onChange,
  providers,
  value,
}: {
  onChange(value: LibraryProviderId): void;
  providers: readonly LibraryProviderDescriptor[];
  value: LibraryProviderId;
}) {
  return (
    <Panel className="library-provider-picker" title="检索后端">
      <div
        className="library-provider-picker__options"
        role="radiogroup"
        aria-label="选择资料库检索后端"
      >
        {providers.map((provider) => {
          const selected = provider.id === value;
          const Icon = provider.id === "sag" ? Waypoints : GitBranch;
          return (
            <button
              aria-checked={selected}
              className={selected ? "is-selected" : undefined}
              key={provider.id}
              onClick={() => onChange(provider.id)}
              role="radio"
              type="button"
            >
              <span className="library-provider-picker__icon">
                <Icon aria-hidden="true" size={18} />
              </span>
              <span>
                <strong>{provider.title}</strong>
                <small>{provider.description}</small>
              </span>
              <Badge variant={selected ? "info" : "neutral"}>
                {selected ? "当前使用" : "可选择"}
              </Badge>
            </button>
          );
        })}
      </div>
    </Panel>
  );
}

function LibraryStatusStrip({ value }: { value: LibraryProviderStatus }) {
  if (isSagStatus(value)) {
    const stats = value.status.stats;
    return (
      <section className="sag-library__stats" aria-label="SAG 索引概览">
        <Stat icon={Database} label="资料源" value={stats.sources ?? 0} />
        <Stat
          icon={FileText}
          label="证据块"
          value={stats.evidence_units ?? 0}
        />
        <Stat icon={Clock3} label="事件" value={stats.events ?? 0} />
        <Stat icon={Layers3} label="实体" value={stats.entities ?? 0} />
        <IndexMeta value={value.status.indexVersion} />
      </section>
    );
  }
  return (
    <section className="sag-library__stats" aria-label="Graph RAG 索引概览">
      <Stat
        icon={Database}
        label="文档（索引总量）"
        value={value.status.documents}
      />
      <Stat icon={FileText} label="分块" value={value.status.chunks} />
      <Stat icon={GitBranch} label="关系" value={value.status.relations} />
      <Stat
        icon={Layers3}
        label="向量后端"
        value={value.status.vectorBackend ?? "未知"}
      />
      <IndexMeta value={value.status.indexVersion} />
    </section>
  );
}

function Stat({
  icon: Icon,
  label,
  value,
}: {
  icon: typeof Database;
  label: string;
  value: number | string;
}) {
  return (
    <article>
      <Icon aria-hidden="true" size={15} />
      <span>{label}</span>
      <strong>{value}</strong>
    </article>
  );
}

function IndexMeta({ value }: { value?: string | null }) {
  return (
    <span className="sag-library__index-meta">
      <small>索引版本</small>
      <code>{value ?? "未知"}</code>
    </span>
  );
}

function LibrarySearchPanel({
  client,
  disabled,
  provider,
}: {
  client: ApiClient | null;
  disabled: boolean;
  provider: LibraryProviderId;
}) {
  const [query, setQuery] = useState("");
  const [namespace, setNamespace] = useState("");
  const [topK, setTopK] = useState(12);
  const [maximumTokens, setMaximumTokens] = useState(5000);
  const [useDeepseek, setUseDeepseek] = useState(true);
  const [retrievalMode, setRetrievalMode] =
    useState<GraphRetrievalMode>("graph");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<LibrarySearchResponse | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!client || disabled || !query.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await client.searchLibrary(provider, {
          query,
          purpose: "evidence_review",
          topK,
          maximumTokens,
          useDeepseek: provider === "sag" && useDeepseek,
          namespaces:
            provider === "sag" && namespace.trim() ? [namespace.trim()] : [],
          retrievalMode: provider === "graph-rag" ? retrievalMode : "auto",
        }),
      );
    } catch (cause) {
      setResult(null);
      setError(sagErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="sag-search">
      <form className="sag-search__query" onSubmit={submit}>
        <label className="sag-search__textarea">
          <span>检索问题</span>
          <textarea
            disabled={disabled || busy}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={
              provider === "sag"
                ? "例如：用户现在对多平台发布格式的偏好是什么？"
                : "例如：从发布规范出发，关联的审核与分发规则是什么？"
            }
            rows={4}
            value={query}
          />
        </label>
        <div className="sag-search__options">
          {provider === "sag" ? (
            <TextField
              disabled={disabled || busy}
              label="命名空间（可选）"
              onChange={(event) => setNamespace(event.target.value)}
              placeholder="enterprise_knowledge"
              value={namespace}
            />
          ) : (
            <div className="sag-library__field">
              <span>检索方式</span>
              <SegmentedControl<GraphRetrievalMode>
                disabled={disabled || busy}
                label="Graph RAG 检索方式"
                onChange={setRetrievalMode}
                options={[
                  { value: "auto", label: "自动" },
                  { value: "hybrid", label: "混合" },
                  { value: "graph", label: "图扩展" },
                ]}
                value={retrievalMode}
              />
            </div>
          )}
          <label className="sag-library__field">
            <span>候选数量</span>
            <NumberField
              disabled={disabled || busy}
              label="候选数量"
              max={30}
              min={1}
              onChange={(value) => setTopK(Math.min(30, Math.max(1, value)))}
              value={topK}
            />
          </label>
          <label className="sag-library__field">
            <span>Context Token 预算</span>
            <NumberField
              disabled={disabled || busy}
              label="Context Token 预算"
              max={16000}
              min={256}
              onChange={(value) =>
                setMaximumTokens(Math.min(16000, Math.max(256, value)))
              }
              unit="tokens"
              value={maximumTokens}
            />
          </label>
          {provider === "sag" ? (
            <label className="sag-library__switch-field">
              <span>
                <strong>DeepSeek 规划</strong>
                <small>拆解证据需求并辅助多路检索</small>
              </span>
              <Switch
                checked={useDeepseek}
                disabled={disabled || busy}
                label="DeepSeek 规划"
                onChange={setUseDeepseek}
              />
            </label>
          ) : (
            <span className="library-provider-hint">
              <GitBranch aria-hidden="true" size={16} />
              <span>
                <strong>关系路径解释</strong>
                <small>展示种子文档、扩展节点与关系类型</small>
              </span>
            </span>
          )}
        </div>
        <Button
          disabled={disabled || busy || !query.trim()}
          type="submit"
          variant="primary"
        >
          <Search aria-hidden="true" size={14} />
          {busy ? "正在检索" : "构造 Context Pack"}
        </Button>
      </form>

      {error ? (
        <p className="sag-library__message is-error" role="alert">
          {error}
        </p>
      ) : null}
      {result ? (
        provider === "sag" && isSagSearchResponse(result) ? (
          <SagSearchResult value={result} />
        ) : !isSagSearchResponse(result) ? (
          <GraphRagSearchResult value={result} />
        ) : null
      ) : (
        <LibrarySearchEmpty provider={provider} />
      )}
    </section>
  );
}

function LibrarySearchEmpty({ provider }: { provider: LibraryProviderId }) {
  return (
    <div className="sag-library__empty">
      <BookOpen aria-hidden="true" size={20} />
      <strong>尚未构造 Context Pack</strong>
      <span>
        {provider === "sag"
          ? "输入问题后，可以查看多路检索命中的证据块及其选择原因。"
          : "输入问题后，可以查看混合检索种子、图扩展证据和关系路径。"}
      </span>
    </div>
  );
}

function SagSearchResult({ value }: { value: SagSearchResponse }) {
  const coverage = coverageByNeed(value.pack.coverage);
  return (
    <section className="sag-result" aria-live="polite">
      <header className="sag-result__summary">
        <SummaryMetric
          label="待审 Context Pack"
          value={`${value.pack.items.length} 个证据块`}
        />
        <SummaryMetric
          label="耗时"
          value={`${value.diagnostics.elapsedSeconds.toFixed(2)} 秒`}
        />
        <SummaryMetric
          label="Token 估算"
          value={`${value.pack.estimatedTokens} / ${value.pack.maximumTokens}`}
        />
        <Badge variant="warning">草稿</Badge>
      </header>

      <Panel
        actions={<Badge variant="neutral">{value.pack.plan.planner}</Badge>}
        title="检索规划 / 证据需求"
      >
        <ol className="sag-result__needs">
          {value.pack.plan.needs.map((need) => {
            const current = coverage.get(need.needId);
            const covered = current?.status === "covered";
            return (
              <li key={need.needId}>
                {covered ? (
                  <CheckCircle2 aria-hidden="true" size={15} />
                ) : (
                  <XCircle aria-hidden="true" size={15} />
                )}
                <span>
                  <strong>{need.description}</strong>
                  <small>
                    {need.query} · {need.timeMode ?? "任意时间"}
                  </small>
                </span>
                <Badge variant={covered ? "success" : "warning"}>
                  {covered ? "已覆盖" : "未覆盖"}
                </Badge>
              </li>
            );
          })}
        </ol>
      </Panel>

      <section className="sag-result__evidence" aria-label="命中证据块">
        {value.pack.items.map((item, index) => (
          <article key={`${item.eventId}:${item.evidenceId}`}>
            <EvidenceHeader
              detail={
                [...item.sectionPath, ...item.anchors]
                  .filter(Boolean)
                  .join(" / ") || item.sourcePath
              }
              index={index}
              score={item.score}
              title={item.title}
            />
            <p className="sag-result__event">{item.eventSummary}</p>
            <blockquote>{item.content}</blockquote>
            <footer>
              <span>来源：{item.sourcePath}</span>
              <span>选择原因：{item.selectionReason}</span>
              <span>约 {item.estimatedTokens} tokens</span>
            </footer>
          </article>
        ))}
      </section>
    </section>
  );
}

function GraphRagSearchResult({ value }: { value: GraphRagSearchResponse }) {
  return (
    <section className="sag-result" aria-live="polite">
      <header className="sag-result__summary">
        <SummaryMetric
          label="待审 Context Pack"
          value={`${value.pack.items.length} 个证据块`}
        />
        <SummaryMetric
          label="图路径"
          value={`${value.pack.graphPaths.length} 条`}
        />
        <SummaryMetric
          label="Token 估算"
          value={`${value.pack.estimatedTokens} / ${value.pack.maximumTokens}`}
        />
        <Badge variant="warning">草稿</Badge>
      </header>

      <Panel
        actions={
          <Badge variant={value.diagnostics.graphUsed ? "info" : "neutral"}>
            {value.diagnostics.graphUsed ? "已执行图扩展" : "仅混合检索"}
          </Badge>
        }
        title="检索判定与图路径"
      >
        <p className="graph-rag-route">
          <strong>{routeLabel(value.pack.route)}</strong>
          <span>{value.pack.routeReason}</span>
        </p>
        {value.pack.graphPaths.length ? (
          <ol className="graph-rag-paths">
            {value.pack.graphPaths.map((path, index) => (
              <li key={`${path.nodeIds.join(":")}:${index}`}>
                <GitBranch aria-hidden="true" size={15} />
                <span>
                  <strong>{path.nodeIds.join(" → ")}</strong>
                  <small>
                    {path.relations.map(relationLabel).join(" → ")} · 置信度{" "}
                    {path.confidence.toFixed(3)}
                  </small>
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="sag-library__message">本次没有产生关系扩展路径。</p>
        )}
      </Panel>

      <section
        className="sag-result__evidence"
        aria-label="Graph RAG 命中证据块"
      >
        {value.pack.items.map((item, index) => (
          <article key={item.itemId}>
            <EvidenceHeader
              detail={[item.documentId, item.sectionTitle, item.anchor]
                .filter(Boolean)
                .join(" / ")}
              index={index}
              score={item.score}
              title={item.title}
            />
            <p className="sag-result__event">
              {item.retrievalMode === "graph"
                ? "图关系扩展命中"
                : "关键词与向量混合命中"}
            </p>
            <blockquote>{item.content}</blockquote>
            <footer>
              <span>关键词分数：{item.lexicalScore.toFixed(3)}</span>
              <span>向量分数：{item.denseScore.toFixed(3)}</span>
              <span>选择原因：{item.selectionReason}</span>
              {item.graphPath.length ? (
                <span>路径：{item.graphPath.join(" → ")}</span>
              ) : null}
            </footer>
          </article>
        ))}
      </section>
    </section>
  );
}

function SummaryMetric({ label, value }: { label: string; value: string }) {
  return (
    <span>
      <small>{label}</small>
      <strong>{value}</strong>
    </span>
  );
}

function EvidenceHeader({
  detail,
  index,
  score,
  title,
}: {
  detail: string;
  index: number;
  score: number;
  title: string;
}) {
  return (
    <header>
      <span className="sag-result__rank">{index + 1}</span>
      <span>
        <strong>{title}</strong>
        <small>{detail}</small>
      </span>
      <Badge variant="info">{score.toFixed(3)}</Badge>
    </header>
  );
}

function LibrarySourcesPanel({
  client,
  disabled,
  provider,
  revision,
}: {
  client: ApiClient | null;
  disabled: boolean;
  provider: LibraryProviderId;
  revision: number;
}) {
  const [query, setQuery] = useState("");
  const [appliedQuery, setAppliedQuery] = useState("");
  const [offset, setOffset] = useState(0);
  const [page, setPage] = useState<LibrarySourcePage | null>(null);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [retryRevision, setRetryRevision] = useState(0);
  const listRef = useRef<HTMLDivElement>(null);
  const pageSize = 100;

  useEffect(() => {
    const timer = window.setTimeout(() => {
      setOffset(0);
      setAppliedQuery(query.trim());
    }, 250);
    return () => window.clearTimeout(timer);
  }, [query]);

  useEffect(() => {
    if (!client || disabled) {
      setPage(null);
      return;
    }
    const controller = new AbortController();
    setLoading(true);
    setLoadError(null);
    void client
      .listLibrarySources(
        provider,
        { query: appliedQuery, offset, limit: pageSize },
        controller.signal,
      )
      .then((nextPage) => {
        if (controller.signal.aborted) return;
        setPage(nextPage);
        listRef.current?.scrollTo({ top: 0 });
      })
      .catch((cause) => {
        if (!controller.signal.aborted) setLoadError(sagErrorMessage(cause));
      })
      .finally(() => {
        if (!controller.signal.aborted) setLoading(false);
      });
    return () => controller.abort();
  }, [
    appliedQuery,
    client,
    disabled,
    offset,
    provider,
    retryRevision,
    revision,
  ]);

  const firstItem = page && page.total ? page.offset + 1 : 0;
  const lastItem = page ? page.offset + page.items.length : 0;
  const countLabel = page
    ? appliedQuery
      ? `${page.total.toLocaleString()} 条匹配`
      : `${page.authorizedTotal.toLocaleString()} 条可见`
    : "读取中";

  return (
    <Panel
      actions={<Badge variant="neutral">{countLabel}</Badge>}
      className="sag-sources"
      title="已加载资料"
    >
      <TextField
        disabled={disabled}
        label="筛选资料"
        onChange={(event) => setQuery(event.target.value)}
        placeholder={
          provider === "sag"
            ? "标题、文件名、命名空间或来源键"
            : "标题、文档 ID、业务分类或负责人"
        }
        type="search"
        value={query}
      />
      {page && provider === "graph-rag" ? (
        <div className="sag-sources__visibility" role="status">
          <ShieldCheck aria-hidden="true" size={15} />
          <span>
            当前身份可见{" "}
            <strong>{page.authorizedTotal.toLocaleString()}</strong> 条； Graph
            RAG 索引总量为 <strong>{page.indexTotal.toLocaleString()}</strong>{" "}
            条。
          </span>
        </div>
      ) : null}
      {disabled ? (
        <p className="sag-library__message">资料库服务连接后可浏览来源。</p>
      ) : loadError ? (
        <div className="sag-library__message is-error" role="alert">
          <span>{loadError}</span>
          <Button
            onClick={() => setRetryRevision((value) => value + 1)}
            size="compact"
            variant="secondary"
          >
            <RefreshCw aria-hidden="true" size={14} /> 重试
          </Button>
        </div>
      ) : loading && !page ? (
        <p className="sag-library__message">正在读取资料列表…</p>
      ) : page?.items.length ? (
        <>
          <div
            aria-busy={loading}
            aria-label={`${providerName(provider)} 资料来源列表`}
            className="sag-sources__list"
            ref={listRef}
            tabIndex={0}
          >
            {page.items.map((source) =>
              isSagSource(source) ? (
                <article key={source.assetId}>
                  <SourceIcon />
                  <span>
                    <strong>{source.title}</strong>
                    <small>
                      {source.originalFilename} · {source.namespace}
                    </small>
                    <code>{source.sourceKey}</code>
                  </span>
                  <span className="sag-sources__counts">
                    <small>{source.evidenceUnits} 证据块</small>
                    <small>{source.events} 事件</small>
                  </span>
                  <Badge variant="neutral">v{source.versionNumber}</Badge>
                </article>
              ) : (
                <article key={source.documentId}>
                  <SourceIcon />
                  <span>
                    <strong>{source.title}</strong>
                    <small>
                      {source.businessClass} · {source.owner}
                    </small>
                    <code>{source.documentId}</code>
                  </span>
                  <span className="sag-sources__counts">
                    <small>{source.sensitivity}</small>
                    <small>{source.sourceUri ?? "无来源地址"}</small>
                  </span>
                  <Badge variant="neutral">v{source.version}</Badge>
                </article>
              ),
            )}
          </div>
          <footer className="sag-sources__pagination">
            <span>
              第 {firstItem.toLocaleString()}–{lastItem.toLocaleString()} 条，共{" "}
              {page.total.toLocaleString()} 条
            </span>
            <span>
              <Button
                disabled={loading || page.offset === 0}
                onClick={() => setOffset(Math.max(0, page.offset - page.limit))}
                size="compact"
                variant="secondary"
              >
                <ChevronLeft aria-hidden="true" size={14} /> 上一页
              </Button>
              <Button
                disabled={loading || !page.hasMore}
                onClick={() => setOffset(page.offset + page.limit)}
                size="compact"
                variant="secondary"
              >
                下一页 <ChevronRight aria-hidden="true" size={14} />
              </Button>
            </span>
          </footer>
        </>
      ) : (
        <div className="sag-library__empty">
          <FileText aria-hidden="true" size={20} />
          <strong>
            {appliedQuery ? "没有匹配资料" : "当前身份没有可见资料"}
          </strong>
          <span>
            {appliedQuery
              ? "换一个筛选条件。"
              : `通过增量导入添加第一份 ${providerName(provider)} 资料。`}
          </span>
        </div>
      )}
    </Panel>
  );
}

function SourceIcon() {
  return (
    <span className="sag-sources__icon">
      <FileText aria-hidden="true" size={16} />
    </span>
  );
}

function LibraryIngestionPanel({
  client,
  disabled,
  onImported,
  provider,
}: {
  client: ApiClient | null;
  disabled: boolean;
  onImported(): Promise<void>;
  provider: LibraryProviderId;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [title, setTitle] = useState("");
  const [namespace, setNamespace] = useState("enterprise_knowledge");
  const [sourceKey, setSourceKey] = useState("");
  const [metadata, setMetadata] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<LibraryIngestionResult | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!client || disabled || !file) return;
    setBusy(true);
    setError(null);
    try {
      const imported = await client.uploadLibrarySource(provider, {
        file,
        title: title.trim() || undefined,
        namespace: namespace.trim() || "enterprise_knowledge",
        sourceKey: sourceKey.trim() || undefined,
        metadata: parseSagMetadata(metadata),
      });
      setResult(imported);
      setFile(null);
      setTitle("");
      setSourceKey("");
      setMetadata("");
      if (fileInput.current) fileInput.current.value = "";
      await onImported();
    } catch (cause) {
      setResult(null);
      setError(sagErrorMessage(cause));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Panel
      className="sag-ingest"
      title={`新增或更新 ${providerName(provider)} 资料`}
    >
      <p className="sag-ingest__intro">
        {provider === "sag"
          ? "相同来源键会创建新版本并切换活动投影，不需要全量重建索引。"
          : "相同文档 ID 会更新对应文档投影，并和关系图使用同一索引版本发布。"}
      </p>
      <form onSubmit={submit}>
        <label className="sag-ingest__file">
          <Upload aria-hidden="true" size={18} />
          <span>
            <strong>{file?.name ?? "选择资料文件"}</strong>
            <small>支持 PDF、DOCX、PPTX、XLSX、HTML、Markdown 和 TXT</small>
          </span>
          <input
            accept=".pdf,.docx,.pptx,.xlsx,.htm,.html,.xhtml,.md,.markdown,.txt"
            disabled={disabled || busy}
            onChange={(event) => setFile(event.target.files?.[0] ?? null)}
            ref={fileInput}
            type="file"
          />
        </label>
        <div className="sag-ingest__fields">
          <TextField
            disabled={disabled || busy}
            hint="留空时使用文件内标题或文件名"
            label="资料标题"
            onChange={(event) => setTitle(event.target.value)}
            value={title}
          />
          <TextField
            disabled={disabled || busy}
            label={provider === "sag" ? "命名空间" : "业务分类"}
            onChange={(event) => setNamespace(event.target.value)}
            required
            value={namespace}
          />
          <TextField
            disabled={disabled || busy}
            hint={
              provider === "sag"
                ? "再次上传相同来源键即执行增量版本更新"
                : "作为稳定文档 ID；再次上传会更新同一文档"
            }
            label={
              provider === "sag" ? "稳定来源键（可选）" : "稳定文档 ID（可选）"
            }
            onChange={(event) => setSourceKey(event.target.value)}
            placeholder={
              provider === "sag" ? "policy/publishing" : "policy-publishing"
            }
            value={sourceKey}
          />
          <TextField
            disabled={disabled || busy}
            hint={
              provider === "sag"
                ? 'JSON 对象，例如 {"department":"sales"}'
                : "可设置 owner、allowed_roles、sensitivity 和 version"
            }
            label="业务元数据（可选）"
            onChange={(event) => setMetadata(event.target.value)}
            value={metadata}
          />
        </div>
        <Button
          disabled={disabled || busy || !file}
          type="submit"
          variant="primary"
        >
          <FilePlus2 aria-hidden="true" size={14} />
          {busy ? "正在分块并构建增量索引" : `导入 ${providerName(provider)}`}
        </Button>
      </form>
      {error ? (
        <p className="sag-library__message is-error" role="alert">
          {error}
        </p>
      ) : null}
      {result ? (
        <p className="sag-library__message is-success" role="status">
          {ingestionMessage(result)}
        </p>
      ) : null}
    </Panel>
  );
}

function providerName(provider: LibraryProviderId): string {
  return provider === "sag" ? "SAG" : "Graph RAG";
}

function isSagStatus(value: LibraryProviderStatus): value is SagLibraryStatus {
  return value.provider === "SAG";
}

function isSagSource(value: LibrarySource): value is SagSource {
  return "assetId" in value;
}

function isSagSearchResponse(
  value: LibrarySearchResponse,
): value is SagSearchResponse {
  return "plan" in value.pack;
}

function ingestionMessage(value: LibraryIngestionResult): string {
  if ("assetId" in value) {
    const sag = value as SagIngestionResult;
    return `${sag.status === "unchanged" ? "资料内容未变化" : "资料已发布"}：${sag.title} · v${sag.versionNumber}`;
  }
  const graph = value as GraphRagIngestionResult;
  return `资料已索引：${graph.title} · v${graph.version} · ${graph.chunkCount} 个分块`;
}

function routeLabel(route: string): string {
  const labels: Record<string, string> = {
    rag: "知识检索",
    exact_search: "精确检索",
    tool: "工具路由（仅用于判定）",
    handoff_or_refuse: "转交或拒答路由（仅用于判定）",
  };
  return labels[route] ?? route;
}

function relationLabel(relation: string): string {
  const labels: Record<string, string> = {
    references: "引用",
    supersedes: "替代",
    related_to: "相关",
  };
  return labels[relation] ?? relation;
}
