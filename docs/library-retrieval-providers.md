# Library retrieval providers

The Flow-mode **Library** surface can switch between SAG and Graph RAG through a
shared provider contract in the local OpenTopia server. OpenTopia starts or
reuses only the provider selected in Library.

This integration is currently review-only: it manages sources and builds draft
Context Packs, but does not inject them into prompts or change the Agent Loop.

## Development configuration

Connect to a source project or an already-running service before starting the
desktop app:

```powershell
# Optional explicit SAG source project
$env:OPENTOPIA_SAG_PROJECT_ROOT="C:\path\to\sag-project"

# Or connect to an externally managed SAG service
$env:OPENTOPIA_SAG_URL="http://127.0.0.1:8765"

# Graph RAG supports the same two options
$env:OPENTOPIA_GRAPH_RAG_PROJECT_ROOT="C:\path\to\graph-rag-project"
$env:OPENTOPIA_GRAPH_RAG_URL="http://127.0.0.1:8000"

pnpm dev:desktop
```

During development, adjacent projects are discovered from the
`enterprise-sag-panel` or `enterprise-graph-rag-panel` entry in
`pyproject.toml`, so directory names are not part of the integration contract.
An explicit project root takes precedence when discovery is not appropriate.

## Packaged and remote providers

A packaged build can provide `OPENTOPIA_SAG_EXECUTABLE` or
`OPENTOPIA_GRAPH_RAG_EXECUTABLE`, or ship the corresponding executable under
`resources/sag/` or `resources/graph-rag/`.

Remote endpoints are never launched by OpenTopia. For a non-development Graph
RAG service, set `OPENTOPIA_GRAPH_RAG_TOKEN` to a service identity token. The
local development handshake otherwise requests a short-lived token using
`OPENTOPIA_GRAPH_RAG_ROLES` and `OPENTOPIA_GRAPH_RAG_TENANT`.

Do not commit service tokens or credentials. Keep local values in an ignored
`.env` file or inject them through the process environment.
