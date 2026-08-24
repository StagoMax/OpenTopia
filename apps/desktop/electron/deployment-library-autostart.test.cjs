const assert = require("node:assert/strict");
const test = require("node:test");

const {
  autostartDeploymentLibraryServices,
  requiredLibraryProvidersFromDeployments,
} = require("./deployment-library-autostart.cjs");

function deployment(status, namespaces) {
  return {
    status,
    snapshot: {
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

test("detects SAG only from active frozen deployment agent specs", () => {
  assert.deepEqual(
    requiredLibraryProvidersFromDeployments([
      deployment("disabled", ["disabled.namespace"]),
      deployment("active", ["audit.namespace"]),
    ]),
    ["sag"],
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
        json: async () => ({ items: [deployment("active", ["audit"])] }),
      };
    },
    ensureProvider: async (provider) => {
      started.push(provider);
      return { state: "ready" };
    },
  });

  assert.deepEqual(providers, ["sag"]);
  assert.deepEqual(started, ["sag"]);
  assert.equal(
    requests[0].url,
    "http://127.0.0.1:8787/api/workflow-deployments",
  );
  assert.equal(
    requests[0].options.headers.authorization,
    "Bearer secret-token",
  );
});
