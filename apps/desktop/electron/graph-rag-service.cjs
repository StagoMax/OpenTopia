const {
  createLibraryProviderServiceManager,
  resolveProviderLaunch,
} = require("./library-provider-service.cjs");

const DEFAULT_GRAPH_RAG_URL = "http://127.0.0.1:8000";
const GRAPH_RAG_SPEC = Object.freeze({
  id: "graph-rag",
  label: "Graph RAG",
  defaultUrl: DEFAULT_GRAPH_RAG_URL,
  urlEnv: "OPENTOPIA_GRAPH_RAG_URL",
  executableEnv: "OPENTOPIA_GRAPH_RAG_EXECUTABLE",
  projectRootEnv: "OPENTOPIA_GRAPH_RAG_PROJECT_ROOT",
  entrypointPattern:
    /enterprise-graph-rag-panel\s*=\s*["']enterprise_rag\.main:main["']/,
  module: "enterprise_rag.main",
  packagedDirectory: "graph-rag",
  packagedExecutable: "enterprise-graph-rag-panel",
  healthPath: "health",
  childEnvPrefixes: ["NOWCODING_", "RAG_"],
  validateHealth: (payload) =>
    payload?.status === "ok" &&
    payload?.prompt_injection === false &&
    payload?.agent_loop_integration === false,
});

function resolveGraphRagLaunch(options = {}) {
  return resolveProviderLaunch({ ...options, spec: GRAPH_RAG_SPEC });
}

function createGraphRagServiceManager(options = {}) {
  return createLibraryProviderServiceManager({
    ...options,
    // 挂载 P3 全量 Milvus 索引时需要重建文档级元数据缓存，首次就绪可能超过 45 秒。
    healthAttempts: options.healthAttempts ?? 180,
    spec: GRAPH_RAG_SPEC,
  });
}

module.exports = {
  DEFAULT_GRAPH_RAG_URL,
  createGraphRagServiceManager,
  resolveGraphRagLaunch,
};
