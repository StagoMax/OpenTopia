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
import { useApplicationLanguage } from "../ApplicationLanguageProvider";
import type { ApplicationLanguage } from "../applicationLanguage";
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
import { libraryMessage, type LibraryMessageKey } from "./libraryPresentation";
import "../styles/sag-library-panel.css";

type LibrarySection = "search" | "sources" | "ingest";
type GraphRetrievalMode = "auto" | "hybrid" | "graph";

const providerStorageKey = "opentopia.library.provider";
function fallbackProviders(
  language: ApplicationLanguage,
): LibraryProviderDescriptor[] {
  const l = (key: LibraryMessageKey) => libraryMessage(language, key);
  return [
    {
      id: "sag",
      name: "SAG",
      title: l("sagTitle"),
      description: l("sagDescription"),
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
      title: l("graphTitle"),
      description: l("graphDescription"),
      capabilities: {
        graphPaths: true,
        temporalMemory: false,
        incrementalUpload: true,
        llmPlanning: false,
      },
    },
  ];
}

function initialProvider(): LibraryProviderId {
  const saved = localStorage.getItem(providerStorageKey);
  return saved === "graph-rag" ? "graph-rag" : "sag";
}

export function LibraryPanel({ client }: { client: ApiClient | null }) {
  const { language, l } = useLibraryLanguage();
  const [provider, setProviderState] =
    useState<LibraryProviderId>(initialProvider);
  const [providers, setProviders] = useState<
    LibraryProviderDescriptor[] | null
  >(null);
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
        setLoadError(l("backendUnavailable"));
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
              `${providerName(provider)} ${l("localServiceUnavailable")}`,
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
    [client, l, provider],
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
    providers?.find((item) => item.id === provider) ??
    fallbackProviders(language)[0];
  const displayedProviders = providers ?? fallbackProviders(language);

  return (
    <section className="sag-library" aria-labelledby="library-title">
      <header className="sag-library__header">
        <span className="sag-library__header-icon">
          <Network aria-hidden="true" size={18} />
        </span>
        <span className="sag-library__heading">
          <small>{l("eyebrow")}</small>
          <strong id="library-title">{l("title")}</strong>
          <span>{l("subtitle")}</span>
        </span>
        <span className="sag-library__header-actions">
          <Badge variant={status ? "success" : loading ? "info" : "warning"}>
            {status ? l("connected") : loading ? l("starting") : l("attention")}
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
            <RefreshCw aria-hidden="true" size={14} /> {l("refresh")}
          </Button>
        </span>
      </header>

      <ProviderPicker
        onChange={setProvider}
        providers={displayedProviders}
        value={provider}
      />

      <section className="sag-library__safety" aria-label={l("boundaryAria")}>
        <ShieldCheck aria-hidden="true" size={16} />
        <span>
          <strong>
            {l("reviewMode")} · {activeDescriptor.name}
          </strong>
          <small>{l("boundaryDetail")}</small>
        </span>
        <Badge variant="neutral">{l("promptOff")}</Badge>
      </section>

      {loadError ? (
        <section className="sag-library__error" role="alert">
          <XCircle aria-hidden="true" size={16} />
          <span>
            <strong>
              {l("cannotConnect")} {providerName(provider)}
            </strong>
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
            {runtime?.canStart ? l("restart") : l("recheck")}
          </Button>
        </section>
      ) : null}

      {status ? <LibraryStatusStrip value={status} /> : null}

      <SegmentedControl<LibrarySection>
        className="sag-library__sections"
        label={`${providerName(provider)} ${l("features")}`}
        onChange={setSection}
        options={[
          { value: "search", label: l("searchReview") },
          { value: "sources", label: l("sources") },
          { value: "ingest", label: l("incrementalImport") },
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
  const { l } = useLibraryLanguage();
  return (
    <Panel className="library-provider-picker" title={l("retrievalBackend")}>
      <div
        className="library-provider-picker__options"
        role="radiogroup"
        aria-label={l("chooseBackendAria")}
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
                {selected ? l("current") : l("available")}
              </Badge>
            </button>
          );
        })}
      </div>
    </Panel>
  );
}

function LibraryStatusStrip({ value }: { value: LibraryProviderStatus }) {
  const { l } = useLibraryLanguage();
  if (isSagStatus(value)) {
    const stats = value.status.stats;
    return (
      <section className="sag-library__stats" aria-label={l("sagOverviewAria")}>
        <Stat
          icon={Database}
          label={l("sourceCount")}
          value={stats.sources ?? 0}
        />
        <Stat
          icon={FileText}
          label={l("evidenceBlocks")}
          value={stats.evidence_units ?? 0}
        />
        <Stat icon={Clock3} label={l("events")} value={stats.events ?? 0} />
        <Stat
          icon={Layers3}
          label={l("entities")}
          value={stats.entities ?? 0}
        />
        <IndexMeta value={value.status.indexVersion} />
      </section>
    );
  }
  return (
    <section className="sag-library__stats" aria-label={l("graphOverviewAria")}>
      <Stat
        icon={Database}
        label={l("indexedDocuments")}
        value={value.status.documents}
      />
      <Stat icon={FileText} label={l("chunks")} value={value.status.chunks} />
      <Stat
        icon={GitBranch}
        label={l("relations")}
        value={value.status.relations}
      />
      <Stat
        icon={Layers3}
        label={l("vectorBackend")}
        value={value.status.vectorBackend ?? l("unknown")}
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
  const { l } = useLibraryLanguage();
  return (
    <span className="sag-library__index-meta">
      <small>{l("indexVersion")}</small>
      <code>{value ?? l("unknown")}</code>
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
  const { l } = useLibraryLanguage();
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
          <span>{l("searchQuestion")}</span>
          <textarea
            disabled={disabled || busy}
            onChange={(event) => setQuery(event.target.value)}
            placeholder={
              provider === "sag"
                ? l("sagQueryPlaceholder")
                : l("graphQueryPlaceholder")
            }
            rows={4}
            value={query}
          />
        </label>
        <div className="sag-search__options">
          {provider === "sag" ? (
            <TextField
              disabled={disabled || busy}
              label={l("namespaceOptional")}
              onChange={(event) => setNamespace(event.target.value)}
              placeholder="enterprise_knowledge"
              value={namespace}
            />
          ) : (
            <div className="sag-library__field">
              <span>{l("retrievalMode")}</span>
              <SegmentedControl<GraphRetrievalMode>
                disabled={disabled || busy}
                label={l("graphRetrievalModeAria")}
                onChange={setRetrievalMode}
                options={[
                  { value: "auto", label: l("automatic") },
                  { value: "hybrid", label: l("hybrid") },
                  { value: "graph", label: l("graphExpansion") },
                ]}
                value={retrievalMode}
              />
            </div>
          )}
          <label className="sag-library__field">
            <span>{l("candidateCount")}</span>
            <NumberField
              disabled={disabled || busy}
              label={l("candidateCount")}
              max={30}
              min={1}
              onChange={(value) => setTopK(Math.min(30, Math.max(1, value)))}
              value={topK}
            />
          </label>
          <label className="sag-library__field">
            <span>{l("contextBudget")}</span>
            <NumberField
              disabled={disabled || busy}
              label={l("contextBudget")}
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
                <strong>{l("deepseekPlanning")}</strong>
                <small>{l("deepseekDetail")}</small>
              </span>
              <Switch
                checked={useDeepseek}
                disabled={disabled || busy}
                label={l("deepseekPlanning")}
                onChange={setUseDeepseek}
              />
            </label>
          ) : (
            <span className="library-provider-hint">
              <GitBranch aria-hidden="true" size={16} />
              <span>
                <strong>{l("pathExplanation")}</strong>
                <small>{l("pathExplanationDetail")}</small>
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
          {busy ? l("searching") : l("buildContextPack")}
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
  const { l } = useLibraryLanguage();
  return (
    <div className="sag-library__empty">
      <BookOpen aria-hidden="true" size={20} />
      <strong>{l("noContextPack")}</strong>
      <span>
        {provider === "sag" ? l("sagEmptyDetail") : l("graphEmptyDetail")}
      </span>
    </div>
  );
}

function SagSearchResult({ value }: { value: SagSearchResponse }) {
  const { l } = useLibraryLanguage();
  const coverage = coverageByNeed(value.pack.coverage);
  return (
    <section className="sag-result" aria-live="polite">
      <header className="sag-result__summary">
        <SummaryMetric
          label={l("reviewContextPack")}
          value={`${value.pack.items.length} ${l("blockUnit")}`}
        />
        <SummaryMetric
          label={l("elapsed")}
          value={`${value.diagnostics.elapsedSeconds.toFixed(2)} ${l("seconds")}`}
        />
        <SummaryMetric
          label={l("tokenEstimate")}
          value={`${value.pack.estimatedTokens} / ${value.pack.maximumTokens}`}
        />
        <Badge variant="warning">{l("draft")}</Badge>
      </header>

      <Panel
        actions={<Badge variant="neutral">{value.pack.plan.planner}</Badge>}
        title={l("retrievalPlan")}
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
                    {need.query} · {need.timeMode ?? l("anyTime")}
                  </small>
                </span>
                <Badge variant={covered ? "success" : "warning"}>
                  {covered ? l("covered") : l("notCovered")}
                </Badge>
              </li>
            );
          })}
        </ol>
      </Panel>

      <section className="sag-result__evidence" aria-label={l("evidenceAria")}>
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
              <span>
                {l("source")}
                {item.sourcePath}
              </span>
              <span>
                {l("selectionReason")}
                {item.selectionReason}
              </span>
              <span>
                {l("approximately")} {item.estimatedTokens} {l("tokens")}
              </span>
            </footer>
          </article>
        ))}
      </section>
    </section>
  );
}

function GraphRagSearchResult({ value }: { value: GraphRagSearchResponse }) {
  const { language, l } = useLibraryLanguage();
  return (
    <section className="sag-result" aria-live="polite">
      <header className="sag-result__summary">
        <SummaryMetric
          label={l("reviewContextPack")}
          value={`${value.pack.items.length} ${l("blockUnit")}`}
        />
        <SummaryMetric
          label={l("graphPaths")}
          value={`${value.pack.graphPaths.length} ${l("pathUnit")}`}
        />
        <SummaryMetric
          label={l("tokenEstimate")}
          value={`${value.pack.estimatedTokens} / ${value.pack.maximumTokens}`}
        />
        <Badge variant="warning">{l("draft")}</Badge>
      </header>

      <Panel
        actions={
          <Badge variant={value.diagnostics.graphUsed ? "info" : "neutral"}>
            {value.diagnostics.graphUsed ? l("graphExpanded") : l("hybridOnly")}
          </Badge>
        }
        title={l("graphDecision")}
      >
        <p className="graph-rag-route">
          <strong>{routeLabel(value.pack.route, language)}</strong>
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
                    {path.relations
                      .map((relation) => relationLabel(relation, language))
                      .join(" → ")}{" "}
                    · {l("confidence")} {path.confidence.toFixed(3)}
                  </small>
                </span>
              </li>
            ))}
          </ol>
        ) : (
          <p className="sag-library__message">{l("noGraphPath")}</p>
        )}
      </Panel>

      <section
        className="sag-result__evidence"
        aria-label={l("graphEvidenceAria")}
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
                ? l("graphMatch")
                : l("hybridMatch")}
            </p>
            <blockquote>{item.content}</blockquote>
            <footer>
              <span>
                {l("lexicalScore")}
                {item.lexicalScore.toFixed(3)}
              </span>
              <span>
                {l("denseScore")}
                {item.denseScore.toFixed(3)}
              </span>
              <span>
                {l("selectionReason")}
                {item.selectionReason}
              </span>
              {item.graphPath.length ? (
                <span>
                  {l("path")}
                  {item.graphPath.join(" → ")}
                </span>
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
  const { language, l } = useLibraryLanguage();
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
      ? `${page.total.toLocaleString(language)} ${l("matchingItems")}`
      : `${page.authorizedTotal.toLocaleString(language)} ${l("visibleItems")}`
    : l("reading");

  return (
    <Panel
      actions={<Badge variant="neutral">{countLabel}</Badge>}
      className="sag-sources"
      title={l("loadedSources")}
    >
      <TextField
        disabled={disabled}
        label={l("filterSources")}
        onChange={(event) => setQuery(event.target.value)}
        placeholder={
          provider === "sag"
            ? l("sagFilterPlaceholder")
            : l("graphFilterPlaceholder")
        }
        type="search"
        value={query}
      />
      {page && provider === "graph-rag" ? (
        <div className="sag-sources__visibility" role="status">
          <ShieldCheck aria-hidden="true" size={15} />
          <span>
            {l("visibleIdentityPrefix")}{" "}
            <strong>{page.authorizedTotal.toLocaleString(language)}</strong>{" "}
            {l("indexTotalPrefix")}{" "}
            <strong>{page.indexTotal.toLocaleString(language)}</strong>
            {l("itemSuffix")}
          </span>
        </div>
      ) : null}
      {disabled ? (
        <p className="sag-library__message">{l("browseAfterConnect")}</p>
      ) : loadError ? (
        <div className="sag-library__message is-error" role="alert">
          <span>{loadError}</span>
          <Button
            onClick={() => setRetryRevision((value) => value + 1)}
            size="compact"
            variant="secondary"
          >
            <RefreshCw aria-hidden="true" size={14} /> {l("retry")}
          </Button>
        </div>
      ) : loading && !page ? (
        <p className="sag-library__message">{l("loadingSources")}</p>
      ) : page?.items.length ? (
        <>
          <div
            aria-busy={loading}
            aria-label={`${providerName(provider)} ${l("sourceList")}`}
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
                    <small>
                      {source.evidenceUnits} {l("evidenceBlocks")}
                    </small>
                    <small>
                      {source.events} {l("events")}
                    </small>
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
                    <small>{source.sourceUri ?? l("noSourceAddress")}</small>
                  </span>
                  <Badge variant="neutral">v{source.version}</Badge>
                </article>
              ),
            )}
          </div>
          <footer className="sag-sources__pagination">
            <span>
              {l("pagePrefix")} {firstItem.toLocaleString(language)}–
              {lastItem.toLocaleString(language)} {l("pageMiddle")}{" "}
              {page.total.toLocaleString(language)} {l("pageSuffix")}
            </span>
            <span>
              <Button
                disabled={loading || page.offset === 0}
                onClick={() => setOffset(Math.max(0, page.offset - page.limit))}
                size="compact"
                variant="secondary"
              >
                <ChevronLeft aria-hidden="true" size={14} /> {l("previous")}
              </Button>
              <Button
                disabled={loading || !page.hasMore}
                onClick={() => setOffset(page.offset + page.limit)}
                size="compact"
                variant="secondary"
              >
                {l("next")} <ChevronRight aria-hidden="true" size={14} />
              </Button>
            </span>
          </footer>
        </>
      ) : (
        <div className="sag-library__empty">
          <FileText aria-hidden="true" size={20} />
          <strong>
            {appliedQuery ? l("noMatches") : l("noVisibleSources")}
          </strong>
          <span>
            {appliedQuery
              ? l("changeFilter")
              : `${l("addFirstPrefix")} ${providerName(provider)} ${l("addFirstSuffix")}`}
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
  const { language, l } = useLibraryLanguage();
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
      title={`${l("upsertTitle")} ${providerName(provider)} ${l("sources")}`}
    >
      <p className="sag-ingest__intro">
        {provider === "sag" ? l("sagIngestDetail") : l("graphIngestDetail")}
      </p>
      <form onSubmit={submit}>
        <label className="sag-ingest__file">
          <Upload aria-hidden="true" size={18} />
          <span>
            <strong>{file?.name ?? l("chooseFile")}</strong>
            <small>{l("supportedFiles")}</small>
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
            hint={l("titleHint")}
            label={l("sourceTitle")}
            onChange={(event) => setTitle(event.target.value)}
            value={title}
          />
          <TextField
            disabled={disabled || busy}
            label={provider === "sag" ? l("namespace") : l("businessClass")}
            onChange={(event) => setNamespace(event.target.value)}
            required
            value={namespace}
          />
          <TextField
            disabled={disabled || busy}
            hint={
              provider === "sag"
                ? l("sagSourceKeyHint")
                : l("graphDocumentIdHint")
            }
            label={
              provider === "sag" ? l("stableSourceKey") : l("stableDocumentId")
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
              provider === "sag" ? l("sagMetadataHint") : l("graphMetadataHint")
            }
            label={l("metadataOptional")}
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
          {busy ? l("indexing") : `${l("import")} ${providerName(provider)}`}
        </Button>
      </form>
      {error ? (
        <p className="sag-library__message is-error" role="alert">
          {error}
        </p>
      ) : null}
      {result ? (
        <p className="sag-library__message is-success" role="status">
          {ingestionMessage(result, language)}
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

function ingestionMessage(
  value: LibraryIngestionResult,
  language: ApplicationLanguage,
): string {
  const l = (key: LibraryMessageKey) => libraryMessage(language, key);
  if ("assetId" in value) {
    const sag = value as SagIngestionResult;
    return `${sag.status === "unchanged" ? l("unchanged") : l("published")}: ${sag.title} · v${sag.versionNumber}`;
  }
  const graph = value as GraphRagIngestionResult;
  return `${l("indexed")}: ${graph.title} · v${graph.version} · ${graph.chunkCount} ${l("chunkUnit")}`;
}

function routeLabel(route: string, language: ApplicationLanguage): string {
  const labels: Record<string, string> = {
    rag: libraryMessage(language, "routeRag"),
    exact_search: libraryMessage(language, "routeExact"),
    tool: libraryMessage(language, "routeTool"),
    handoff_or_refuse: libraryMessage(language, "routeHandoff"),
  };
  return labels[route] ?? route;
}

function relationLabel(
  relation: string,
  language: ApplicationLanguage,
): string {
  const labels: Record<string, string> = {
    references: libraryMessage(language, "relationReferences"),
    supersedes: libraryMessage(language, "relationSupersedes"),
    related_to: libraryMessage(language, "relationRelated"),
  };
  return labels[relation] ?? relation;
}

function useLibraryLanguage() {
  const { language } = useApplicationLanguage();
  const l = useCallback(
    (key: LibraryMessageKey) => libraryMessage(language, key),
    [language],
  );
  return { language, l };
}
