function deploymentItems(payload) {
  if (Array.isArray(payload)) return payload;
  return Array.isArray(payload?.items) ? payload.items : [];
}

function requiredLibraryProvidersFromDeployments(payload) {
  const providers = new Set();
  for (const deployment of deploymentItems(payload)) {
    if (deployment?.status !== "active") continue;
    const revision = deployment.activeRevision || deployment.snapshot;
    for (const agent of Object.values(
      revision?.compiledWorkflow?.agentSpecs || {},
    )) {
      const binding = agent?.knowledgeBinding;
      if (!binding) continue;
      // Namespace-only bindings predate provider selection and are SAG.
      const provider = binding.provider || "sag";
      if (["sag", "graph-rag"].includes(provider)) providers.add(provider);
    }
  }
  return ["sag", "graph-rag"].filter((provider) => providers.has(provider));
}

async function autostartDeploymentLibraryServices({
  backendUrl,
  apiToken,
  ensureProvider,
  fetchImpl = fetch,
}) {
  const response = await fetchImpl(
    `${String(backendUrl).replace(/\/$/, "")}/api/flows?status=active`,
    { headers: { authorization: `Bearer ${apiToken}` } },
  );
  if (!response.ok) {
    throw new Error(
      `Active Flow library discovery failed with HTTP ${response.status}`,
    );
  }

  const providers = requiredLibraryProvidersFromDeployments(
    await response.json(),
  );
  for (const provider of providers) {
    const status = await ensureProvider(provider);
    if (status?.state === "unavailable") {
      throw new Error(status.message || `${provider} service is unavailable`);
    }
  }
  return providers;
}

module.exports = {
  autostartDeploymentLibraryServices,
  requiredLibraryProvidersFromDeployments,
};
