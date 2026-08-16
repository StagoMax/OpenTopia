const {
  createLibraryProviderServiceManager,
  discoverProviderProject,
  endpointInfo: genericEndpointInfo,
  providerChildEnv,
  resolveProviderLaunch,
} = require("./library-provider-service.cjs");

const DEFAULT_SAG_URL = "http://127.0.0.1:8765";
const SAG_SPEC = Object.freeze({
  id: "sag",
  label: "SAG",
  defaultUrl: DEFAULT_SAG_URL,
  urlEnv: "OPENTOPIA_SAG_URL",
  executableEnv: "OPENTOPIA_SAG_EXECUTABLE",
  projectRootEnv: "OPENTOPIA_SAG_PROJECT_ROOT",
  entrypointPattern:
    /enterprise-sag-panel\s*=\s*["']enterprise_sag\.panel:main["']/,
  module: "enterprise_sag.panel",
  packagedDirectory: "sag",
  packagedExecutable: "enterprise-sag-panel",
  healthPath: "api/status",
  childEnvPrefixes: ["OPENTOPIA_SAG_", "SAG_"],
  validateHealth: (payload) =>
    (payload?.status === "ready" || payload?.status === "ok") &&
    payload?.prompt_injection === false &&
    payload?.agent_loop_integration === false,
});

function endpointInfo(endpoint = DEFAULT_SAG_URL) {
  return genericEndpointInfo(endpoint, SAG_SPEC);
}

function sagChildEnv(env) {
  return providerChildEnv(env, SAG_SPEC);
}

function discoverSagProject(searchRoot) {
  return discoverProviderProject(searchRoot, SAG_SPEC);
}

function resolveSagLaunch(options = {}) {
  return resolveProviderLaunch({ ...options, spec: SAG_SPEC });
}

function createSagServiceManager(options = {}) {
  return createLibraryProviderServiceManager({ ...options, spec: SAG_SPEC });
}

module.exports = {
  DEFAULT_SAG_URL,
  createSagServiceManager,
  discoverSagProject,
  endpointInfo,
  resolveSagLaunch,
  sagChildEnv,
};
