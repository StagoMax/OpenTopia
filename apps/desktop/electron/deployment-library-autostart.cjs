function deploymentItems(payload) {
  if (Array.isArray(payload)) return payload;
  return Array.isArray(payload?.items) ? payload.items : [];
}

function requiredLibraryProvidersFromDeployments(payload) {
  const requiresSag = deploymentItems(payload)
    .filter((deployment) => deployment?.status === "active")
    .some((deployment) =>
      Object.values(
        deployment?.snapshot?.compiledWorkflow?.agentSpecs || {},
      ).some(
        (agent) =>
          Array.isArray(agent?.knowledgeBinding?.namespaces) &&
          agent.knowledgeBinding.namespaces.length > 0,
      ),
    );
  return requiresSag ? ["sag"] : [];
}

async function autostartDeploymentLibraryServices({
  backendUrl,
  apiToken,
  ensureProvider,
  fetchImpl = fetch,
}) {
  const response = await fetchImpl(
    `${String(backendUrl).replace(/\/$/, "")}/api/workflow-deployments`,
    { headers: { authorization: `Bearer ${apiToken}` } },
  );
  if (!response.ok) {
    throw new Error(
      `Deployment library discovery failed with HTTP ${response.status}`,
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
