[CmdletBinding()]
param(
  [string]$ServerPath = "",
  [int]$Port = 8884,
  [string]$ArtifactRoot = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$verificationId = "flow-enterprise-{0}" -f ([guid]::NewGuid().ToString("N"))
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

$databasePath = Join-Path $ArtifactRoot "opentopia-e2e.db"
$stdoutPath = Join-Path $ArtifactRoot "server.stdout.log"
$stderrPath = Join-Path $ArtifactRoot "server.stderr.log"
$apiToken = "opentopia-enterprise-e2e-token-0123456789abcdef0123456789"
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
    $parameters.Body = $Body | ConvertTo-Json -Depth 60 -Compress
  }
  Invoke-RestMethod @parameters
}

function Invoke-TopiaExpectedError {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][object]$Body,
    [Parameter(Mandatory = $true)][int]$StatusCode
  )
  try {
    Invoke-TopiaApi POST $Path $Body | Out-Null
  } catch {
    $actual = [int]$_.Exception.Response.StatusCode
    if ($actual -eq $StatusCode) { return }
    throw "Expected HTTP $StatusCode from $Path, received $actual"
  }
  throw "Expected HTTP $StatusCode from $Path, but request succeeded"
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
    Start-Sleep -Milliseconds 150
  } while ([DateTime]::UtcNow -lt $deadline)
  $snapshot = $run | ConvertTo-Json -Depth 20 -Compress
  throw "Flow Run $RunId did not reach: $($Statuses -join ', '); last state: $snapshot"
}

function New-AgentTemplateSpec {
  param([string]$Instructions)
  @{
    description = $Instructions
    instructions = $Instructions
    capabilities = @{
      allowAllTools = $false; tools = @()
      allowAllSkills = $false; skills = @()
      allowAllPlugins = $false; plugins = @()
      allowAllMcpServers = $false; mcpServers = @()
      allowAllWorkspaceRoots = $false; workspaceRoots = @($repoRoot)
    }
    connectionBindings = @()
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

function New-GraphNode {
  param([string]$Id, [string]$Label, [string]$Kind, [hashtable]$Config)
  @{ id = $Id; label = $Label; kind = $Kind; config = $Config; inputSchema = @{ type = "object" }; outputSchema = @{ type = "object" } }
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
  Invoke-TopiaApi PATCH "/api/settings" @{
    providerKind = "mock"
    model = "flow-enterprise-e2e"
  } | Out-Null

  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "P3.1 and P4 background verification"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }

  $draftA = Invoke-TopiaApi POST "/api/agent-templates" @{
    templateId = "enterprise-intake-$verificationId"
    name = "Enterprise intake"
    owner = "opentopia-e2e"
    spec = New-AgentTemplateSpec "Normalize the request into a compact JSON object."
  }
  $templateA = Invoke-TopiaApi POST "/api/agent-templates/$($draftA.template.templateId)/versions/$($draftA.template.version)/publish" @{
    approvedBy = "opentopia-e2e"
    approveCapabilityExpansion = $true
  }
  $draftB = Invoke-TopiaApi POST "/api/agent-templates" @{
    templateId = "enterprise-review-$verificationId"
    name = "Enterprise reviewer"
    owner = "opentopia-e2e"
    spec = New-AgentTemplateSpec "Review the normalized request and return one JSON decision."
  }
  $templateB = Invoke-TopiaApi POST "/api/agent-templates/$($draftB.template.templateId)/versions/$($draftB.template.version)/publish" @{
    approvedBy = "opentopia-e2e"
    approveCapabilityExpansion = $true
  }

  $instanceA = Invoke-TopiaApi POST "/api/agent-instances" @{
    templateId = $templateA.template.templateId
    templateVersion = $templateA.template.version
    threadId = $thread.id
    initialState = @{}
    bindToThread = $false
  }
  $instanceB = Invoke-TopiaApi POST "/api/agent-instances" @{
    templateId = $templateB.template.templateId
    templateVersion = $templateB.template.version
    threadId = $thread.id
    initialState = @{}
    bindToThread = $false
  }

  $schema = @{ type = "object" }
  $flowSpec = @{
    flowId = "enterprise-surface-$verificationId"
    name = "Enterprise surface verification"
    description = "Execute two independently frozen Agent templates and review the output"
    owner = "opentopia-e2e"
    categories = @("runtime-verification")
    source = @{ kind = "natural_language"; description = "P3.1 and P4 real verification" }
    inputSchema = $schema
    outputSchema = $schema
    graph = @{
      schemaVersion = 1
      entryNodeId = "intake"
      nodes = @(
        (New-GraphNode "intake" "Intake Agent" "agent" @{ reference = $templateA.template.templateId; templateVersion = $templateA.template.version }),
        (New-GraphNode "review" "Review Agent" "agent" @{ reference = $templateB.template.templateId; templateVersion = $templateB.template.version }),
        (New-GraphNode "output" "Inbox output" "output" @{})
      )
      edges = @(
        @{ from = "intake"; to = "review"; allowedFields = @(); dataClassification = "internal" },
        @{ from = "review"; to = "output"; allowedFields = @(); dataClassification = "internal" }
      )
    }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 8; maxToolCalls = 8; maxDurationSeconds = 180; maxLoopIterations = 2 }
    riskClass = "low"
    pendingDecisions = @()
  }
  $flowDraft = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $flowSpec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) { throw "Workflow validation failed" }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/simulate" @{ input = @{ request = "verify enterprise surface" } }
  if ($trial.status -ne "passed") { throw "Workflow Trial failed" }
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($flowDraft.draft.id)/publish" @{ publishedBy = "opentopia-e2e" }

  Invoke-TopiaExpectedError "/api/threads/$($thread.id)/flow-runs" @{ flowId = $definition.flowId; version = $definition.version; input = @{} } 400

  $deployment = Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $definition.flowId
    flowVersion = $definition.version
    name = "P3.1 frozen Agent deployment"
    environment = "e2e"
    createdBy = "opentopia-e2e"
  }
  $agentSpecs = @($deployment.snapshot.compiledWorkflow.agentSpecs.PSObject.Properties | ForEach-Object { $_.Value })
  if ($agentSpecs.Count -ne 2) { throw "DeploymentSnapshot did not freeze two WorkflowAgentSpecs" }
  $instructions = @($agentSpecs | ForEach-Object { $_.instructions } | Sort-Object -Unique)
  if ($instructions.Count -ne 2) { throw "WorkflowAgentSpecs did not preserve independent instructions" }
  foreach ($agentSpec in $agentSpecs) {
    if ($agentSpec.connectionAuthority.mode -ne "structured") {
      throw "Workflow Agent $($agentSpec.nodeId) did not freeze structured authority"
    }
  }

  $started = Invoke-TopiaApi POST "/api/threads/$($thread.id)/workflow-deployments/$($deployment.id)/runs" @{
    input = @{ request = "verify frozen per-node Agent identity" }
  }
  $run = Wait-ForRunStatus $started.id @("waiting_human", "succeeded", "failed", "cancelled")
  if ($run.status -eq "waiting_human") {
    $tasks = Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($run.id)"
    if ($tasks.Count -ne 1 -or $tasks[0].taskType -ne "output_review") {
      throw "Expected one output review task after Agent nodes"
    }
    $claimed = Invoke-TopiaApi POST "/api/human-tasks/$($tasks[0].id)/claim" @{ expectedRevision = $tasks[0].revision }
    Invoke-TopiaApi POST "/api/human-tasks/$($claimed.id)/resolve" @{
      expectedRevision = $claimed.revision
      action = "approve"
      note = "P3.1 background verification"
      idempotencyKey = "p31-output-$verificationId"
    } | Out-Null
    $run = Wait-ForRunStatus $run.id @("succeeded", "failed", "cancelled")
  }
  if ($run.status -ne "succeeded") {
    throw "Deployed Agent workflow did not succeed: $($run.status) $($run.error)"
  }
  if (@($run.nodeRuns | Where-Object { $_.nodeId -in @("intake", "review") -and $_.status -eq "succeeded" }).Count -ne 2) {
    throw "Both independently frozen Agent nodes did not succeed"
  }

  $globalAgents = Invoke-TopiaApi GET "/api/agent-instances?limit=20"
  $globalRuns = Invoke-TopiaApi GET "/api/flow-runs?limit=20"
  $templates = Invoke-TopiaApi GET "/api/agent-templates"
  $workflows = Invoke-TopiaApi GET "/api/flows"
  $deployments = Invoke-TopiaApi GET "/api/workflow-deployments"
  $connections = Invoke-TopiaApi GET "/api/connections"
  $pendingTasks = Invoke-TopiaApi GET "/api/human-tasks?status=pending"
  if ($globalAgents.Count -lt 2 -or $globalRuns.Count -lt 1 -or $templates.Count -lt 2 -or $workflows.Count -lt 1 -or $deployments.Count -lt 1) {
    throw "Enterprise Surface global queries did not return the persisted objects: agents=$($globalAgents.Count), runs=$($globalRuns.Count), templates=$($templates.Count), workflows=$($workflows.Count), deployments=$($deployments.Count)"
  }

  $result = [ordered]@{
    verifiedAt = [DateTime]::UtcNow.ToString("o")
    service = "opentopia-server"
    transport = "background-http-api"
    foregroundAutomation = $false
    threadId = $thread.id
    agentInstanceIds = @($instanceA.instance.id, $instanceB.instance.id)
    workflowDefinition = "$($definition.flowId)@$($definition.version)"
    deploymentId = $deployment.id
    deploymentSnapshotHash = $deployment.snapshot.contentHash
    frozenAgentNodes = @($agentSpecs | ForEach-Object { "$($_.nodeId):$($_.templateId)@$($_.templateVersion)" })
    runId = $run.id
    runStatus = $run.status
    successfulAgentNodes = @($run.nodeRuns | Where-Object { $_.nodeId -in @("intake", "review") -and $_.status -eq "succeeded" }).Count
    globalAgents = $globalAgents.Count
    globalRuns = $globalRuns.Count
    templates = $templates.Count
    workflows = $workflows.Count
    deployments = $deployments.Count
    connections = $connections.Count
    pendingHumanTasks = $pendingTasks.Count
    directMutableAgentRunRejected = $true
  }
  $resultPath = Join-Path $ArtifactRoot "result.json"
  $result | ConvertTo-Json -Depth 30 | Set-Content -LiteralPath $resultPath -Encoding UTF8
  $result | ConvertTo-Json -Depth 30
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id
    Wait-Process -Id $server.Id -Timeout 10 -ErrorAction SilentlyContinue
  }
  foreach ($entry in $environment.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $previousEnvironment[$entry.Key], "Process")
  }
}
