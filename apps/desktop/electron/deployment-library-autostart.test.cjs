const assert = require("node:assert/strict");
const test = require("node:test");

const {
  autostartDeploymentLibraryServices,
  requiredLibraryProvidersFromDeployments,
} = require("./deployment-library-autostart.cjs");

function deployment(status, namespaces, libraryProvider) {
  return {
    status,
    activeRevision: {
      ...(libraryProvider ? { libraryProvider } : {}),
      compiledWorkflow: {
        agentSpecs: {
          evidence: namespaces
            ? { knowledgeBinding: { namespaces } }
            : { connectionBindings: [] },
        },
      },
    },
  };
}

test("detects providers from active frozen Flow revisions", () => {
  assert.deepEqual(
    requiredLibraryProvidersFromDeployments([
      deployment("disabled", ["disabled.namespace"]),
      deployment("active", ["audit.namespace"]),
      deployment("active", null, "graph-rag"),
    ]),
    ["sag", "graph-rag"],
  );
  assert.deepEqual(
    requiredLibraryProvidersFromDeployments([deployment("active")]),
    [],
  );
});

test("starts required providers after authenticated deployment discovery", async () => {
  const requests = [];
  const started = [];
  const providers = await autostartDeploymentLibraryServices({
    backendUrl: "http://127.0.0.1:8787/",
    apiToken: "secret-token",
    fetchImpl: async (url, options) => {
      requests.push({ url, options });
      return {
        ok: true,
        json: async () => ({
          items: [deployment("active", null, "graph-rag")],
        }),
      };
    },
    ensureProvider: async (provider) => {
      started.push(provider);
      return { state: "ready" };
    },
  });

  assert.deepEqual(providers, ["graph-rag"]);
  assert.deepEqual(started, ["graph-rag"]);
  assert.equal(
    requests[0].url,
    "http://127.0.0.1:8787/api/flows?status=active",
  );
  assert.equal(
    requests[0].options.headers.authorization,
    "Bearer secret-token",
  );
});
