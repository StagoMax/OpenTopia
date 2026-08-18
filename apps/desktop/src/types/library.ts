import type { LibraryProviderId } from "./platform";

export type SagLibraryStatus = {
  provider: "SAG";
  endpoint: string;
  status: {
    status: string;
    database?: string | null;
    indexVersion?: string | null;
    embeddingBackend?: string | null;
    embeddingDimensions?: number | null;
    stats: Record<string, number>;
    integrityCheck?: string | null;
    modelLoaded: boolean;
    deepseekConfigured: boolean;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type SagSource = {
  assetId: string;
  sourceKey: string;
  namespace: string;
  origin: string;
  versionId: string;
  versionNumber: number;
  sourceId: string;
  title: string;
  originalFilename: string;
  contentHash: string;
  storedPath: string;
  metadata: Record<string, unknown>;
  evidenceUnits: number;
  events: number;
  createdAt: string;
};

export type SagSearchRequest = {
  query: string;
  purpose?: string;
  topK?: number;
  maximumTokens?: number;
  useDeepseek?: boolean;
  subjectRefs?: string[];
  namespaces?: string[];
};

export type SagEvidenceNeed = {
  needId: string;
  description: string;
  query: string;
  facets: string[];
  subjectRefs: string[];
  timeMode?: string | null;
  required: boolean;
  weight: number;
};

export type SagNeedCoverage = {
  needId: string;
  required: boolean;
  status: "covered" | "uncovered" | string;
  selectedEventIds: string[];
  reason: string;
};

export type SagContextPackItem = {
  eventId: string;
  evidenceId: string;
  content: string;
  eventSummary: string;
  sourcePath: string;
  title: string;
  sectionPath: string[];
  anchors: string[];
  score: number;
  selectionReason: string;
  matchedNeedIds: string[];
  estimatedTokens: number;
};

export type SagSearchResponse = {
  pack: {
    packId?: string | null;
    status: "draft" | "approved" | "rejected" | string;
    purpose?: string | null;
    query?: string | null;
    plan: {
      requestId?: string | null;
      originalQuery?: string | null;
      purpose?: string | null;
      planner: string;
      needs: SagEvidenceNeed[];
      createdAt?: string | null;
    };
    coverage: SagNeedCoverage[];
    indexVersion?: string | null;
    retrievalEngine?: string | null;
    items: SagContextPackItem[];
    excludedItems: unknown[];
    estimatedTokens: number;
    maximumTokens: number;
    createdAt?: string | null;
  };
  diagnostics: {
    elapsedSeconds: number;
    routeCandidates: Record<string, number>;
    llmRequests: number;
    embeddingBackend?: string | null;
    deepseekEnabled: boolean;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type SagIngestionResult = {
  jobId: string;
  status: "published" | "unchanged" | string;
  assetId: string;
  versionId: string;
  previousVersionId?: string | null;
  versionNumber: number;
  sourceId: string;
  contentHash: string;
  namespace: string;
  title: string;
  storedPath: string;
  indexVersion: string;
  pipelineSignature: string;
  reusedProjection: boolean;
  evidenceUnits: number;
  events: number;
  entities: number;
  llmRequests: number;
  createdAt: string;
};

export type LibraryProviderDescriptor = {
  id: LibraryProviderId;
  name: string;
  title: string;
  description: string;
  capabilities: {
    graphPaths: boolean;
    temporalMemory: boolean;
    incrementalUpload: boolean;
    llmPlanning: boolean;
  };
};

export type GraphRagLibraryStatus = {
  provider: "Graph RAG";
  endpoint: string;
  status: {
    status: string;
    embeddingBackend?: string | null;
    embeddingDimensions?: number | null;
    rerankerBackend?: string | null;
    vectorBackend?: string | null;
    documents: number;
    chunks: number;
    relations: number;
    indexVersion?: string | null;
    graphEnabled: boolean;
    stats: Record<string, number>;
    agentLoopIntegration: boolean;
    promptInjection: boolean;
  };
};

export type LibraryProviderStatus = SagLibraryStatus | GraphRagLibraryStatus;

export type GraphRagSource = {
  documentId: string;
  title: string;
  owner: string;
  businessClass: string;
  sensitivity: string;
  version: string;
  sourceUri?: string | null;
};

export type LibrarySource = SagSource | GraphRagSource;

export type LibrarySourcePage = {
  items: LibrarySource[];
  total: number;
  authorizedTotal: number;
  indexTotal: number;
  offset: number;
  limit: number;
  hasMore: boolean;
};

export type LibrarySearchRequest = SagSearchRequest & {
  retrievalMode?: "auto" | "hybrid" | "graph";
};

export type GraphRagContextPackItem = {
  itemId: string;
  chunkId: string;
  documentId: string;
  title: string;
  content: string;
  anchor: string;
  sectionTitle?: string | null;
  score: number;
  lexicalScore: number;
  denseScore: number;
  retrievalMode: "hybrid" | "graph";
  graphPath: string[];
  graphRelations: string[];
  selectionReason: string;
  estimatedTokens: number;
};

export type GraphRagSearchResponse = {
  pack: {
    packId: string;
    status: "draft";
    query: string;
    route: string;
    routeReason: string;
    indexVersion: string;
    retrievalEngine: "graph_rag";
    items: GraphRagContextPackItem[];
    graphPaths: Array<{
      nodeIds: string[];
      relations: string[];
      confidence: number;
    }>;
    estimatedTokens: number;
    maximumTokens: number;
    createdAt: string;
  };
  diagnostics: {
    hitCount: number;
    graphUsed: boolean;
    graphPathCount: number;
    embeddingBackend: string;
    agentLoopIntegration: false;
    promptInjection: false;
  };
};

export type LibrarySearchResponse = SagSearchResponse | GraphRagSearchResponse;

export type GraphRagIngestionResult = {
  status: "indexed" | string;
  documentId: string;
  sourceKey: string;
  namespace: string;
  title: string;
  originalFilename: string;
  version: string;
  chunkCount: number;
  indexVersion: string;
  contentHash: string;
};

export type LibraryIngestionResult =
  SagIngestionResult | GraphRagIngestionResult;
