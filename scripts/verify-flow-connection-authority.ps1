[CmdletBinding()]
param(
  [string]$ServerPath = "",
  [int]$Port = 8891,
  [string]$ArtifactRoot = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$verificationId = "flow-connection-{0}" -f ([guid]::NewGuid().ToString("N"))
if (-not $ArtifactRoot) {
  $ArtifactRoot = Join-Path $repoRoot ".opentopia\evaluations\$verificationId"
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

$fixturePath = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot "flow-mcp-fixture.mjs"))
$nodePath = (Get-Command node -ErrorAction Stop).Source
$databasePath = Join-Path $ArtifactRoot "opentopia-e2e.db"
$stdoutPath = Join-Path $ArtifactRoot "server.stdout.log"
$stderrPath = Join-Path $ArtifactRoot "server.stderr.log"
$apiToken = "opentopia-flow-connection-token-0123456789abcdef0123456789"
$baseUrl = "http://127.0.0.1:$Port"
$headers = @{ Authorization = "Bearer $apiToken" }
$environment = @{
  OPENTOPIA_API_TOKEN = $apiToken
  OPENTOPIA_PLUGIN_HOME = (Join-Path $ArtifactRoot "plugins")
  OPENTOPIA_BUNDLED_PLUGIN_HOME = (Join-Path $ArtifactRoot "bundled-plugins")
  OPENTOPIA_RUNTIME_HOME = (Join-Path $ArtifactRoot "runtime")
  OPENTOPIA_ARTIFACTS_DIR = (Join-Path $ArtifactRoot "artifacts")
  OPENTOPIA_ENTERPRISE_ENABLED = "true"
  OPENTOPIA_OFFICE_RUNTIME_AUTO_INSTALL = "false"
}
$previousEnvironment = @{}
foreach ($entry in $environment.GetEnumerator()) {
  $previousEnvironment[$entry.Key] = [Environment]::GetEnvironmentVariable($entry.Key, "Process")
  [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, "Process")
}

function Invoke-TopiaApi {
  param(
    [Parameter(Mandatory = $true)][ValidateSet("GET", "POST", "PATCH")][string]$Method,
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
    $parameters.Body = $Body | ConvertTo-Json -Depth 80 -Compress
  }
  Invoke-RestMethod @parameters
}

function Invoke-TopiaExpectedError {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][object]$Body,
    [Parameter(Mandatory = $true)][int[]]$StatusCodes
  )
  try {
    Invoke-TopiaApi POST $Path $Body | Out-Null
  } catch {
    $actual = [int]$_.Exception.Response.StatusCode
    if ($actual -in $StatusCodes) { return $actual }
    throw "Expected HTTP $($StatusCodes -join ' or ') from $Path, received $actual"
  }
  throw "Expected HTTP $($StatusCodes -join ' or ') from $Path, but request succeeded"
}

function Wait-ForHealth {
  param([Diagnostics.Process]$Process)
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    if ($Process.HasExited) {
      throw "OpenTopia server exited before health check; inspect $stderrPath"
    }
    try {
      $health = Invoke-RestMethod -Method Get -Uri "$baseUrl/health" -Headers $headers -TimeoutSec 2
      if ($health.ok) { return }
    } catch {
      Start-Sleep -Milliseconds 250
    }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "OpenTopia server did not become healthy"
}

function Wait-ForRunStatus {
  param([string]$RunId, [string[]]$Statuses)
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  do {
    $run = Invoke-TopiaApi GET "/api/flow-runs/$RunId"
    if ($run.status -in $Statuses) { return $run }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Flow Run $RunId did not reach $($Statuses -join ', ')"
}

function Complete-FlowRun {
  param([object]$Run)
  if ($Run.status -eq "waiting_human") {
    $tasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($Run.id)")
    if ($tasks.Count -ne 1 -or $tasks[0].taskType -ne "output_review") {
      throw "Expected one output review task"
    }
    Invoke-TopiaApi POST "/api/human-tasks/$($tasks[0].id)/resolve" @{
      expectedRevision = $tasks[0].revision
      action = "approve"
      idempotencyKey = "connection-output-$verificationId"
    } | Out-Null
    return Wait-ForRunStatus $Run.id @("succeeded", "failed", "cancelled")
  }
  return $Run
}

function New-AgentTemplateSpec {
  param([string]$ConnectionId, [int]$CapabilityRevision, [string]$OperationId)
  @{
    description = "Use one account-scoped customer lookup operation."
    instructions = "Return a compact customer verification result."
    capabilities = @{
      allowAllTools = $false; tools = @()
      allowAllSkills = $false; skills = @()
      allowAllPlugins = $false; plugins = @()
      allowAllMcpServers = $false; mcpServers = @()
      allowAllWorkspaceRoots = $false; workspaceRoots = @($repoRoot)
    }
    connectionBindings = @(@{
      connectionId = $ConnectionId
      capabilityRevision = $CapabilityRevision
      operationGrants = @(@{ operationId = $OperationId })
    })
    resourceGrants = @()
    modelPolicy = @{ allowAllModels = $true; allowedModels = @() }
    stateSchema = @{ type = "object" }
    outputSchema = @{ type = "object" }
    allowAllDelegates = $false
    delegateTemplateIds = @()
    budget = @{ maxTurns = 6; maxToolCalls = 4; maxDurationSeconds = 120 }
    riskClass = "low"
  }
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
  Wait-ForHealth $server
  Invoke-TopiaApi PATCH "/api/settings" @{ providerKind = "mock"; model = "flow-connection-e2e" } | Out-Null

  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "Flow Connection authority background verification"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }
  $mcp = Invoke-TopiaApi POST "/api/mcp/servers" @{
    name = "Flow fixture CRM"
    command = $nodePath
    args = @($fixturePath)
    cwd = $repoRoot
    envKeys = @()
    timeoutMs = 15000
    enabled = $true
  }
  if ($mcp.status.status -ne "ready" -or $mcp.status.toolsCount -ne 1) {
    throw "Fixture MCP runtime did not become ready"
  }
  $connectionId = $mcp.server.serverId
  $refresh = Invoke-TopiaApi POST "/api/connections/$connectionId/capabilities/refresh"
  $capabilities = @($refresh.capabilityRevision.capabilities)
  if ($refresh.connection.status -ne "ready" -or $capabilities.Count -ne 1) {
    throw "Connection capability discovery did not publish one ready operation"
  }
  $operation = $capabilities[0]
  if ($operation.providerMetadata.toolName -ne "lookup_customer") {
    throw "Unexpected MCP capability projection"
  }

  $draft = Invoke-TopiaApi POST "/api/agent-templates" @{
    templateId = "flow-connection-$verificationId"
    name = "Flow customer lookup"
    owner = "opentopia-e2e"
    spec = New-AgentTemplateSpec $connectionId $refresh.capabilityRevision.revision $operation.capabilityId
  }
  $template = Invoke-TopiaApi POST "/api/agent-templates/$($draft.template.templateId)/versions/$($draft.template.version)/publish" @{
    approvedBy = "opentopia-e2e"
    approveCapabilityExpansion = $true
  }
  $instance = Invoke-TopiaApi POST "/api/agent-instances" @{
    templateId = $template.template.templateId
    templateVersion = $template.template.version
    threadId = $thread.id
    initialState = @{}
    bindToThread = $true
  }
  if (-not $instance.bound) { throw "Structured Agent instance was not bound to the Flow thread" }

  $call = Invoke-TopiaApi POST "/api/mcp/servers/$connectionId/call-tool" @{
    toolName = "lookup_customer"
    arguments = @{ customerId = "C-42" }
    threadId = $thread.id
  }
  if ($call.isError -or $call.structuredContent.customerId -ne "C-42") {
    throw "Structured Connection operation did not return the fixture customer"
  }

  $schema = @{ type = "object" }
  $flowSpec = @{
    flowId = "flow-connection-authority-$verificationId"
    name = "Flow Connection authority verification"
    description = "Freeze and enforce one Agent-owned Connection operation"
    owner = "opentopia-e2e"
    categories = @("runtime-verification")
    source = @{ kind = "natural_language"; description = "Connection authority background verification" }
    inputSchema = $schema
    outputSchema = $schema
    graph = @{
      schemaVersion = 1
      entryNodeId = "approve"
      nodes = @(
        @{ id = "approve"; label = "Approve lookup"; kind = "approval"; config = @{}; inputSchema = $schema; outputSchema = $schema },
        @{ id = "lookup"; label = "Lookup Agent"; kind = "agent"; config = @{ reference = $template.template.templateId; templateVersion = $template.template.version }; inputSchema = $schema; outputSchema = $schema },
        @{ id = "output"; label = "Output"; kind = "output"; config = @{}; inputSchema = $schema; outputSchema = $schema }
      )
      edges = @(
        @{ from = "approve"; to = "lookup"; allowedFields = @(); dataClassification = "internal" },
        @{ from = "lookup"; to = "output"; allowedFields = @(); dataClassification = "internal" }
      )
    }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 4; maxToolCalls = 4; maxDurationSeconds = 120; maxLoopIterations = 1 }
    riskClass = "low"
    pendingDecisions = @()
  }
  $flowDraft = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $flowSpec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) { throw "Connection workflow validation failed" }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/simulate" @{ input = @{ customerId = "C-42" } }
  if ($trial.status -ne "passed") { throw "Connection workflow trial failed" }
  $testStarted = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/test-run" @{
    input = @{ customerId = "C-42-test-run" }
    startedBy = "opentopia-e2e"
  }
  $testPaused = Wait-ForRunStatus $testStarted.id @("waiting_approval", "failed", "cancelled")
  if ($testPaused.status -ne "waiting_approval") { throw "Connection Flow Test Run did not reach approval" }
  $testTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($testStarted.id)")
  if ($testTasks.Count -ne 1 -or $testTasks[0].taskType -ne "approval") {
    throw "Connection Flow Test Run did not create one approval task"
  }
  Invoke-TopiaApi POST "/api/human-tasks/$($testTasks[0].id)/resolve" @{
    expectedRevision = $testTasks[0].revision
    action = "approve"
    idempotencyKey = "connection-test-run-$verificationId"
  } | Out-Null
  $testRun = Complete-FlowRun (Wait-ForRunStatus $testStarted.id @("waiting_human", "succeeded", "failed", "cancelled"))
  if ($testRun.status -ne "succeeded") { throw "Connection Flow Test Run failed" }
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/publish" @{ publishedBy = "opentopia-e2e" }
  $deployment = Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $definition.flowId
    flowVersion = $definition.version
    name = "Connection authority deployment"
    environment = "e2e"
    createdBy = "opentopia-e2e"
  }
  $agentSpec = $deployment.snapshot.compiledWorkflow.agentSpecs.lookup
  if ($agentSpec.connectionAuthority.mode -ne "structured" -or @($agentSpec.connectionAuthority.operations).Count -ne 1) {
    throw "Deployment did not freeze the one structured Connection operation"
  }
  $started = Invoke-TopiaApi POST "/api/threads/$($thread.id)/workflow-deployments/$($deployment.id)/runs" @{
    input = @{ customerId = "C-42" }
  }
  $authorizedApprovalRun = Wait-ForRunStatus $started.id @("waiting_approval", "failed", "cancelled")
  if ($authorizedApprovalRun.status -ne "waiting_approval") { throw "Authorized Flow did not reach approval" }
  $authorizedApprovalTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($started.id)")
  if ($authorizedApprovalTasks.Count -ne 1 -or $authorizedApprovalTasks[0].taskType -ne "approval") {
    throw "Expected one authorized approval task"
  }
  Invoke-TopiaApi POST "/api/human-tasks/$($authorizedApprovalTasks[0].id)/resolve" @{
    expectedRevision = $authorizedApprovalTasks[0].revision
    action = "approve"
    idempotencyKey = "connection-authorized-approval-$verificationId"
  } | Out-Null
  $run = Complete-FlowRun (Wait-ForRunStatus $started.id @("waiting_human", "succeeded", "failed", "cancelled"))
  if ($run.status -ne "succeeded") { throw "Authorized Flow run did not succeed" }

  # Pause a second run before its Agent node. Revoking the Connection must
  # leave both this run and its Human task retryable when approval is attempted.
  $recoveryStarted = Invoke-TopiaApi POST "/api/threads/$($thread.id)/workflow-deployments/$($deployment.id)/runs" @{
    input = @{ customerId = "C-44" }
  }
  $recoveryWaiting = Wait-ForRunStatus $recoveryStarted.id @("waiting_approval", "failed", "cancelled")
  if ($recoveryWaiting.status -ne "waiting_approval") { throw "Recovery Flow did not reach approval" }
  $recoveryApprovalTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($recoveryStarted.id)")
  if ($recoveryApprovalTasks.Count -ne 1 -or $recoveryApprovalTasks[0].taskType -ne "approval") {
    throw "Expected one recovery approval task"
  }
  $recoveryApprovalTask = $recoveryApprovalTasks[0]

  $disabled = Invoke-TopiaApi PATCH "/api/connections/$connectionId" @{
    expectedRevision = $refresh.connection.revision
    enabled = $false
  }
  if ($disabled.status -ne "disabled") { throw "Connection did not become disabled" }
  $revokedDirectStatus = Invoke-TopiaExpectedError "/api/mcp/servers/$connectionId/call-tool" @{
    toolName = "lookup_customer"
    arguments = @{ customerId = "C-43" }
    threadId = $thread.id
  } @(403, 409)
  $runsBeforeRejectedStart = @(Invoke-TopiaApi GET "/api/threads/$($thread.id)/flow-runs")
  $revokedRunStatusCode = Invoke-TopiaExpectedError "/api/threads/$($thread.id)/workflow-deployments/$($deployment.id)/runs" @{
    input = @{ customerId = "C-43" }
  } @(400, 403, 409)
  $runsAfterRejectedStart = @(Invoke-TopiaApi GET "/api/threads/$($thread.id)/flow-runs")
  if ($runsAfterRejectedStart.Count -ne $runsBeforeRejectedStart.Count) {
    throw "Rejected deployment start persisted an orphan Flow Run"
  }
  $revokedApprovalStatusCode = Invoke-TopiaExpectedError "/api/human-tasks/$($recoveryApprovalTask.id)/resolve" @{
    expectedRevision = $recoveryApprovalTask.revision
    action = "approve"
    idempotencyKey = "connection-revoked-approval-$verificationId"
  } @(409)
  $recoveryAfterRejection = Invoke-TopiaApi GET "/api/flow-runs/$($recoveryStarted.id)"
  $pendingAfterRejection = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($recoveryStarted.id)")
  if ($recoveryAfterRejection.status -ne "waiting_approval" -or $pendingAfterRejection.Count -ne 1) {
    throw "Rejected approval did not preserve the retryable Flow and Human task state"
  }

  $reenabled = Invoke-TopiaApi PATCH "/api/connections/$connectionId" @{
    expectedRevision = $disabled.revision
    enabled = $true
  }
  $retested = Invoke-TopiaApi POST "/api/connections/$connectionId/test"
  if (-not $retested.health.ok -or $retested.connection.status -ne "ready") {
    throw "Connection did not recover after re-enabling"
  }
  Invoke-TopiaApi POST "/api/human-tasks/$($recoveryApprovalTask.id)/resolve" @{
    expectedRevision = $recoveryApprovalTask.revision
    action = "approve"
    idempotencyKey = "connection-revoked-approval-$verificationId"
  } | Out-Null
  $recoveryRun = Complete-FlowRun (Wait-ForRunStatus $recoveryStarted.id @("waiting_human", "succeeded", "failed", "cancelled"))
  if ($recoveryRun.status -ne "succeeded") { throw "Recovered approval Flow did not succeed" }

  $result = [ordered]@{
    verifiedAt = [DateTime]::UtcNow.ToString("o")
    service = "opentopia-server"
    transport = "background-http-api-plus-stdio-mcp"
    foregroundAutomation = $false
    threadId = $thread.id
    connectionId = $connectionId
    capabilityRevision = $refresh.capabilityRevision.revision
    operationId = $operation.capabilityId
    agentInstanceId = $instance.instance.id
    deploymentId = $deployment.id
    testRunId = $testRun.id
    authorizedRunId = $run.id
    authorizedRunStatus = $run.status
    structuredCallCustomerId = $call.structuredContent.customerId
    threadMcpToggleRequired = $false
    revokedDirectCallRejected = $true
    revokedDirectCallStatusCode = $revokedDirectStatus
    revokedRunId = $null
    revokedRunStatus = "rejected_before_start"
    revokedRunStatusCode = $revokedRunStatusCode
    rejectedStartPersistedRun = $false
    revokedApprovalStatusCode = $revokedApprovalStatusCode
    revokedApprovalPreservedRunStatus = $recoveryAfterRejection.status
    revokedApprovalPreservedPendingTasks = $pendingAfterRejection.Count
    recoveredRunId = $recoveryRun.id
    recoveredRunStatus = $recoveryRun.status
    frozenOperationCount = @($agentSpec.connectionAuthority.operations).Count
  }
  $resultPath = Join-Path $ArtifactRoot "result.json"
  $result | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $resultPath -Encoding UTF8
  $result | ConvertTo-Json -Depth 30
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    $server.WaitForExit()
  }
  foreach ($entry in $previousEnvironment.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, "Process")
  }
}
