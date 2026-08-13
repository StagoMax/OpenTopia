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
  Clock3,
  Database,
  FilePlus2,
  FileText,
  Layers3,
  Network,
  RefreshCw,
  Search,
  ShieldCheck,
  Upload,
  XCircle,
} from "lucide-react";
import type { ApiClient } from "../api/client";
import {
  coverageByNeed,
  filterSagSources,
  parseSagMetadata,
  sagErrorMessage,
} from "../sagLibrary";
import { ensureSagLibraryService } from "../platform";
import type {
  SagIngestionResult,
  SagLibraryStatus,
  SagSearchResponse,
  SagServiceRuntimeStatus,
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

export function SagLibraryPanel({ client }: { client: ApiClient | null }) {
  const [section, setSection] = useState<LibrarySection>("search");
  const [status, setStatus] = useState<SagLibraryStatus | null>(null);
  const [sources, setSources] = useState<SagSource[]>([]);
  const [loading, setLoading] = useState(true);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [runtime, setRuntime] = useState<SagServiceRuntimeStatus | null>(null);

  const refresh = useCallback(
    async (signal?: AbortSignal) => {
      if (!client) {
        setStatus(null);
        setSources([]);
        setLoadError("OpenTopia 服务尚未连接。请稍后重试。");
        setLoading(false);
        return;
      }
      setLoading(true);
      setLoadError(null);
      try {
        const runtimeStatus = await ensureSagLibraryService();
        if (signal?.aborted) return;
        setRuntime(runtimeStatus);
        if (runtimeStatus?.state === "unavailable") {
          throw new Error(runtimeStatus.message || "SAG 本地服务尚未就绪。");
        }
        const [nextStatus, nextSources] = await Promise.all([
          client.getSagLibraryStatus(signal),
          client.listSagSources(signal),
        ]);
        setStatus(nextStatus);
        setSources(nextSources);
      } catch (cause) {
        if (signal?.aborted) return;
        setStatus(null);
        setSources([]);
        setLoadError(sagErrorMessage(cause));
      } finally {
        if (!signal?.aborted) setLoading(false);
      }
    },
    [client],
  );

  useEffect(() => {
    const controller = new AbortController();
    void refresh(controller.signal);
    return () => controller.abort();
  }, [refresh]);

  return (
    <section className="sag-library" aria-labelledby="sag-library-title">
      <header className="sag-library__header">
        <span className="sag-library__header-icon">
          <Network aria-hidden="true" size={18} />
        </span>
        <span className="sag-library__heading">
          <small>Library / 资料库</small>
          <strong id="sag-library-title">SAG 记忆与知识检索</strong>
          <span>增量资料管理、多路检索与 Context Pack 人工审阅</span>
        </span>
        <span className="sag-library__header-actions">
          <Badge variant={status ? "success" : loading ? "info" : "warning"}>
            {status ? "服务已连接" : loading ? "正在启动" : "需要处理"}
          </Badge>
          <Button
            disabled={loading}
            onClick={() => void refresh()}
            size="compact"
            variant="quiet"
          >
            <RefreshCw aria-hidden="true" size={14} /> 刷新
          </Button>
        </span>
      </header>

      <section className="sag-library__safety" aria-label="集成边界">
        <ShieldCheck aria-hidden="true" size={16} />
        <span>
          <strong>审阅模式</strong>
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
            <strong>无法连接 SAG</strong>
            <small>{loadError}</small>
            {runtime?.endpoint ? <code>{runtime.endpoint}</code> : null}
          </span>
          <Button
            disabled={loading}
            onClick={() => void refresh()}
            size="compact"
            variant="secondary"
          >
            <RefreshCw aria-hidden="true" size={14} />
            {runtime?.canStart ? "重新启动" : "重新检测"}
          </Button>
        </section>
      ) : null}

      {status ? <SagStatusStrip value={status} /> : null}

      <SegmentedControl<LibrarySection>
        className="sag-library__sections"
        label="SAG 资料库功能"
        onChange={setSection}
        options={[
          { value: "search", label: "检索审阅" },
          { value: "sources", label: `资料来源 ${sources.length}` },
          { value: "ingest", label: "增量导入" },
        ]}
        value={section}
      />

      {section === "search" ? (
        <SagSearchPanel client={client} disabled={!status} />
      ) : section === "sources" ? (
        <SagSourcesPanel loading={loading} sources={sources} />
      ) : (
        <SagIngestionPanel
          client={client}
          disabled={!status}
          onImported={async () => {
            await refresh();
            setSection("sources");
          }}
        />
      )}
    </section>
  );
}

function SagStatusStrip({ value }: { value: SagLibraryStatus }) {
  const stats = value.status.stats;
  return (
    <section className="sag-library__stats" aria-label="SAG 索引概览">
      <article>
        <Database aria-hidden="true" size={15} />
        <span>资料源</span>
        <strong>{stats.sources ?? 0}</strong>
      </article>
      <article>
        <FileText aria-hidden="true" size={15} />
        <span>证据块</span>
        <strong>{stats.evidence_units ?? 0}</strong>
      </article>
      <article>
        <Clock3 aria-hidden="true" size={15} />
        <span>事件</span>
        <strong>{stats.events ?? 0}</strong>
      </article>
      <article>
        <Layers3 aria-hidden="true" size={15} />
        <span>实体</span>
        <strong>{stats.entities ?? 0}</strong>
      </article>
      <span className="sag-library__index-meta">
        <small>索引版本</small>
        <code>{value.status.indexVersion ?? "未知"}</code>
      </span>
    </section>
  );
}

function SagSearchPanel({
  client,
  disabled,
}: {
  client: ApiClient | null;
  disabled: boolean;
}) {
  const [query, setQuery] = useState("");
  const [namespace, setNamespace] = useState("");
  const [topK, setTopK] = useState(12);
  const [maximumTokens, setMaximumTokens] = useState(5000);
  const [useDeepseek, setUseDeepseek] = useState(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SagSearchResponse | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!client || disabled || !query.trim()) return;
    setBusy(true);
    setError(null);
    try {
      setResult(
        await client.searchSag({
          query,
          purpose: "evidence_review",
          topK,
          maximumTokens,
          useDeepseek,
          namespaces: namespace.trim() ? [namespace.trim()] : [],
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
            placeholder="例如：用户现在对多平台发布格式的偏好是什么？"
            rows={4}
            value={query}
          />
        </label>
        <div className="sag-search__options">
          <TextField
            disabled={disabled || busy}
            label="命名空间（可选）"
            onChange={(event) => setNamespace(event.target.value)}
            placeholder="enterprise_knowledge"
            value={namespace}
          />
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
      {result ? <SagSearchResult value={result} /> : <SagSearchEmpty />}
    </section>
  );
}

function SagSearchEmpty() {
  return (
    <div className="sag-library__empty">
      <BookOpen aria-hidden="true" size={20} />
      <strong>尚未构造 Context Pack</strong>
      <span>输入问题后，可以查看多路检索命中的证据块及其选择原因。</span>
    </div>
  );
}

function SagSearchResult({ value }: { value: SagSearchResponse }) {
  const coverage = coverageByNeed(value.pack.coverage);
  return (
    <section className="sag-result" aria-live="polite">
      <header className="sag-result__summary">
        <span>
          <small>待审 Context Pack</small>
          <strong>{value.pack.items.length} 个证据块</strong>
        </span>
        <span>
          <small>耗时</small>
          <strong>{value.diagnostics.elapsedSeconds.toFixed(2)} 秒</strong>
        </span>
        <span>
          <small>Token 估算</small>
          <strong>
            {value.pack.estimatedTokens} / {value.pack.maximumTokens}
          </strong>
        </span>
        <Badge variant="warning">{value.pack.status}</Badge>
      </header>

      <Panel
        actions={<Badge variant="neutral">{value.pack.plan.planner}</Badge>}
        title="检索规划 / Evidence Needs"
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
                    {need.query} · {need.timeMode ?? "any"}
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
            <header>
              <span className="sag-result__rank">{index + 1}</span>
              <span>
                <strong>{item.title}</strong>
                <small>
                  {[...item.sectionPath, ...item.anchors]
                    .filter(Boolean)
                    .join(" / ") || item.sourcePath}
                </small>
              </span>
              <Badge variant="info">{item.score.toFixed(3)}</Badge>
            </header>
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

function SagSourcesPanel({
  loading,
  sources,
}: {
  loading: boolean;
  sources: readonly SagSource[];
}) {
  const [query, setQuery] = useState("");
  const visible = useMemo(
    () => filterSagSources(sources, query),
    [sources, query],
  );
  return (
    <Panel
      actions={<Badge variant="neutral">{visible.length} 项</Badge>}
      className="sag-sources"
      title="已加载资料"
    >
      <TextField
        label="筛选资料"
        onChange={(event) => setQuery(event.target.value)}
        placeholder="标题、文件名、命名空间或来源键"
        type="search"
        value={query}
      />
      {loading ? (
        <p className="sag-library__message">正在读取资料列表…</p>
      ) : visible.length ? (
        <div className="sag-sources__list">
          {visible.map((source) => (
            <article key={source.assetId}>
              <span className="sag-sources__icon">
                <FileText aria-hidden="true" size={16} />
              </span>
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
          ))}
        </div>
      ) : (
        <div className="sag-library__empty">
          <FileText aria-hidden="true" size={20} />
          <strong>{sources.length ? "没有匹配资料" : "尚未加载资料"}</strong>
          <span>
            {sources.length
              ? "换一个标题、文件名、命名空间或来源键。"
              : "通过增量导入添加第一份企业资料。"}
          </span>
        </div>
      )}
    </Panel>
  );
}

function SagIngestionPanel({
  client,
  disabled,
  onImported,
}: {
  client: ApiClient | null;
  disabled: boolean;
  onImported(): Promise<void>;
}) {
  const fileInput = useRef<HTMLInputElement>(null);
  const [file, setFile] = useState<File | null>(null);
  const [title, setTitle] = useState("");
  const [namespace, setNamespace] = useState("enterprise_knowledge");
  const [sourceKey, setSourceKey] = useState("");
  const [metadata, setMetadata] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<SagIngestionResult | null>(null);

  async function submit(event: FormEvent) {
    event.preventDefault();
    if (!client || disabled || !file) return;
    setBusy(true);
    setError(null);
    try {
      const imported = await client.uploadSagSource({
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
    <Panel className="sag-ingest" title="新增或更新资料">
      <p className="sag-ingest__intro">
        每次只处理本次上传的资料。相同来源键会创建新版本并切换活动投影，不需要全量重建索引。
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
            label="命名空间"
            onChange={(event) => setNamespace(event.target.value)}
            required
            value={namespace}
          />
          <TextField
            disabled={disabled || busy}
            hint="再次上传相同来源键即执行增量版本更新"
            label="稳定来源键（可选）"
            onChange={(event) => setSourceKey(event.target.value)}
            placeholder="policy/publishing"
            value={sourceKey}
          />
          <TextField
            disabled={disabled || busy}
            hint='JSON 对象，例如 {"department":"sales"}'
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
          {busy ? "正在分块并构建增量索引" : "导入 SAG"}
        </Button>
      </form>
      {error ? (
        <p className="sag-library__message is-error" role="alert">
          {error}
        </p>
      ) : null}
      {result ? (
        <p className="sag-library__message is-success" role="status">
          {result.status === "unchanged" ? "资料内容未变化" : "资料已发布"}：
          {result.title} · v{result.versionNumber}
        </p>
      ) : null}
    </Panel>
  );
}
