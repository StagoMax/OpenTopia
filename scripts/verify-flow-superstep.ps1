[CmdletBinding()]
param(
  [string]$ServerPath = "",
  [int]$Port = 8879,
  [string]$ArtifactRoot = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$runId = "flow-superstep-{0}" -f ([guid]::NewGuid().ToString("N"))
if (-not $ArtifactRoot) {
  $ArtifactRoot = Join-Path $repoRoot ".opentopia\evaluations\$runId"
}
$ArtifactRoot = [IO.Path]::GetFullPath($ArtifactRoot)
New-Item -ItemType Directory -Force -Path $ArtifactRoot | Out-Null

if (-not $ServerPath) {
  $ServerPath = Join-Path $repoRoot "target\debug\opentopia-server.exe"
}
$ServerPath = [IO.Path]::GetFullPath($ServerPath)
if (-not (Test-Path -LiteralPath $ServerPath -PathType Leaf)) {
  throw "OpenTopia server binary not found: $ServerPath"
}

$databasePath = Join-Path $ArtifactRoot "opentopia-e2e.db"
$stdoutPath = Join-Path $ArtifactRoot "server.stdout.log"
$stderrPath = Join-Path $ArtifactRoot "server.stderr.log"
$pluginHome = Join-Path $ArtifactRoot "plugins"
$bundledPluginHome = Join-Path $ArtifactRoot "bundled-plugins"
$runtimeHome = Join-Path $ArtifactRoot "runtime"
$artifactsHome = Join-Path $ArtifactRoot "artifacts"
$apiToken = "opentopia-flow-e2e-token-0123456789abcdef0123456789abcdef"
$baseUrl = "http://127.0.0.1:$Port"
$headers = @{ Authorization = "Bearer $apiToken" }

$environmentNames = @(
  "OPENTOPIA_API_TOKEN",
  "OPENTOPIA_PLUGIN_HOME",
  "OPENTOPIA_BUNDLED_PLUGIN_HOME",
  "OPENTOPIA_RUNTIME_HOME",
  "OPENTOPIA_ARTIFACTS_DIR",
  "OPENTOPIA_ENTERPRISE_ENABLED",
  "OPENTOPIA_OFFICE_RUNTIME_AUTO_INSTALL"
)
$previousEnvironment = @{}
foreach ($name in $environmentNames) {
  $previousEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}
[Environment]::SetEnvironmentVariable("OPENTOPIA_API_TOKEN", $apiToken, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_PLUGIN_HOME", $pluginHome, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_BUNDLED_PLUGIN_HOME", $bundledPluginHome, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_RUNTIME_HOME", $runtimeHome, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_ARTIFACTS_DIR", $artifactsHome, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_ENTERPRISE_ENABLED", "true", "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_OFFICE_RUNTIME_AUTO_INSTALL", "false", "Process")

function Invoke-TopiaApi {
  param(
    [Parameter(Mandatory = $true)][ValidateSet("GET", "POST", "PUT", "PATCH")][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [object]$Body = $null
  )
  $parameters = @{
    Method = $Method
    Uri = "$baseUrl$Path"
    Headers = $headers
    TimeoutSec = 30
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json; charset=utf-8"
    $parameters.Body = $Body | ConvertTo-Json -Depth 40 -Compress
  }
  Invoke-RestMethod @parameters
}

function Wait-ForTopiaHealth {
  param([Diagnostics.Process]$Process)
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    if ($Process.HasExited) {
      throw "OpenTopia server exited before health check; inspect $stderrPath"
    }
    try {
      $health = Invoke-RestMethod -Method Get -Uri "$baseUrl/health" -Headers $headers -TimeoutSec 2
      if ($health.ok -and $health.service -eq "opentopia-server") {
        return
      }
    } catch {
      Start-Sleep -Milliseconds 250
    }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "OpenTopia server did not become healthy within 90 seconds"
}

$server = $null
try {
  $server = Start-Process `
    -FilePath $ServerPath `
    -ArgumentList @("--host", "127.0.0.1", "--port", $Port, "--db", $databasePath, "--permission", "full-access") `
    -WorkingDirectory $repoRoot `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru `
    -WindowStyle Hidden
  Wait-ForTopiaHealth $server

  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "Phase 2B real superstep verification"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }

  $objectSchema = @{ type = "object" }
  $nodes = @(
    @{ id = "entry"; label = "Entry"; kind = "condition"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "left"; label = "Left validator"; kind = "validator"; config = @{ stateWrites = @(@{ channel = "results"; reducer = "append" }) }; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "right"; label = "Right validator"; kind = "validator"; config = @{ stateWrites = @(@{ channel = "results"; reducer = "append" }) }; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "join"; label = "Join"; kind = "join"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "output"; label = "Output"; kind = "output"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema }
  )
  $edges = @(
    @{ from = "entry"; to = "left"; allowedFields = @(); dataClassification = "internal" },
    @{ from = "entry"; to = "right"; allowedFields = @(); dataClassification = "internal" },
    @{ from = "left"; to = "join"; allowedFields = @(); dataClassification = "internal" },
    @{ from = "right"; to = "join"; allowedFields = @(); dataClassification = "internal" },
    @{ from = "join"; to = "output"; allowedFields = @(); dataClassification = "internal" }
  )
  $flowSpec = @{
    flowId = "phase2b-real-superstep"
    name = "Phase 2B real superstep"
    description = "Background API verification for state channels and checkpoint commit"
    owner = "opentopia-e2e"
    categories = @("runtime-verification")
    source = @{ kind = "natural_language"; description = "deterministic verification" }
    inputSchema = $objectSchema
    outputSchema = $objectSchema
    graph = @{ schemaVersion = 1; entryNodeId = "entry"; nodes = $nodes; edges = $edges }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 20; maxToolCalls = 1; maxDurationSeconds = 60; maxLoopIterations = 2 }
    riskClass = "low"
    pendingDecisions = @()
  }

  $draftView = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $flowSpec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($draftView.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) {
    throw "Flow validation failed: $($validated.draft.lastValidation.issues | ConvertTo-Json -Depth 12 -Compress)"
  }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($draftView.draft.id)/simulate" @{
    input = @{ requestId = $runId; ready = $true }
  }
  if ($trial.status -ne "passed") {
    throw "Flow simulation failed: $($trial | ConvertTo-Json -Depth 12 -Compress)"
  }
  $testStarted = Invoke-TopiaApi POST "/api/flow-drafts/$($draftView.draft.id)/test-run" @{
    input = @{ requestId = $runId; ready = $true }
    startedBy = "opentopia-e2e"
  }
  $testDeadline = [DateTime]::UtcNow.AddSeconds(30)
  $testRun = $testStarted
  do {
    $testRun = Invoke-TopiaApi GET "/api/flow-runs/$($testStarted.id)"
    if ($testRun.status -in @("succeeded", "failed", "cancelled")) { break }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $testDeadline)
  if ($testRun.status -ne "succeeded") {
    throw "Workflow Test Run did not succeed: status=$($testRun.status), error=$($testRun.error)"
  }
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($draftView.draft.id)/publish" @{ publishedBy = "opentopia-e2e" }
  $deployment = Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $definition.flowId
    flowVersion = $definition.version
    name = "Phase 2B background deployment"
    environment = "e2e"
    createdBy = "opentopia-e2e"
    outputReviewPolicy = "explicit_nodes_only"
  }
  $release = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "phase2b-$runId"
    environment = "e2e"
    threadId = $thread.id
    deploymentId = $deployment.id
    trigger = @{
      kind = "event_subscription"
      triggerId = [guid]::NewGuid().ToString()
      source = "opentopia-e2e"
      eventType = "flow.requested"
    }
    ingressPolicy = "require_review"
    createdBy = "opentopia-e2e"
  }
  $pendingResults = @(Invoke-TopiaApi POST "/api/workflow-events" @{
    source = "opentopia-e2e"
    eventType = "flow.requested"
    idempotencyKey = $runId
    payload = @{ requestId = $runId; ready = $true }
  })
  if ($pendingResults.Count -ne 1 -or $pendingResults[0].invocation.status -ne "accepted" -or $pendingResults[0].run) {
    throw "Reviewed event did not stop in the pending invocation queue"
  }
  $pendingInvocations = @(Invoke-TopiaApi GET "/api/workflow-trigger-invocations?releaseId=$($release.id)")
  if ($pendingInvocations.Count -ne 1 -or $pendingInvocations[0].input.requestId -ne $runId) {
    throw "Pending event input was not durably queryable"
  }
  $approved = Invoke-TopiaApi POST "/api/workflow-trigger-invocations/$($pendingInvocations[0].id)/start" @{}
  if (-not $approved.run -or $approved.invocation.status -ne "started") {
    throw "Approved event did not create a Flow Run"
  }
  $started = $approved.run

  $deadline = [DateTime]::UtcNow.AddSeconds(30)
  $run = $started
  do {
    $run = Invoke-TopiaApi GET "/api/flow-runs/$($started.id)"
    if ($run.status -in @("succeeded", "failed", "cancelled")) { break }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)

  if ($run.status -ne "succeeded") {
    throw "Workflow Run did not succeed: status=$($run.status), error=$($run.error)"
  }
  $parallelCheckpoint = @($run.checkpointHistory | Where-Object {
      $_.nodeIds.Count -eq 2 -and $_.nodeIds[0] -eq "left" -and $_.nodeIds[1] -eq "right"
    })[0]
  if (-not $parallelCheckpoint) {
    throw "No committed left/right parallel checkpoint was found"
  }
  if ($parallelCheckpoint.status -ne "committed" -or $parallelCheckpoint.pendingWriteCount -ne 2) {
    throw "Parallel checkpoint did not atomically commit both pending writes"
  }
  if (@($run.state.results).Count -ne 2) {
    throw "Append reducer did not produce two deterministic state values"
  }
  if ($null -ne $run.activeCheckpoint) {
    throw "Succeeded Run still has an active checkpoint"
  }

  $result = [ordered]@{
    verifiedAt = [DateTime]::UtcNow.ToString("o")
    service = "opentopia-server"
    transport = "background-http-api"
    threadId = $thread.id
    definitionId = $definition.id
    deploymentId = $deployment.id
    releaseId = $release.id
    invocationId = $approved.invocation.id
    testRunId = $testRun.id
    runId = $run.id
    status = $run.status
    superstep = $run.superstep
    checkpointCount = @($run.checkpointHistory).Count
    parallelCheckpoint = $parallelCheckpoint
    state = $run.state
    nodeAttempts = @($run.nodeRuns).Count
  }
  $resultPath = Join-Path $ArtifactRoot "result.json"
  $result | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $resultPath -Encoding UTF8
  $result | ConvertTo-Json -Depth 30
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id
    Wait-Process -Id $server.Id -Timeout 10 -ErrorAction SilentlyContinue
  }
  foreach ($name in $environmentNames) {
    [Environment]::SetEnvironmentVariable($name, $previousEnvironment[$name], "Process")
  }
}
