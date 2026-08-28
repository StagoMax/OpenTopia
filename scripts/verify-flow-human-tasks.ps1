[CmdletBinding()]
param(
  [string]$ServerPath = "",
  [int]$Port = 8881,
  [string]$ArtifactRoot = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$verificationId = "flow-human-tasks-{0}" -f ([guid]::NewGuid().ToString("N"))
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
$apiToken = "opentopia-human-task-e2e-token-0123456789abcdef0123456789"
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
    $parameters.Body = $Body | ConvertTo-Json -Depth 50 -Compress
  }
  Invoke-RestMethod @parameters
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
  $deadline = [DateTime]::UtcNow.AddSeconds(30)
  do {
    $run = Invoke-TopiaApi GET "/api/flow-runs/$RunId"
    if ($run.status -in $Statuses) { return $run }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Flow Run $RunId did not reach: $($Statuses -join ', ')"
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

  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "Phase 3 real HumanTask verification"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }
  $schema = @{ type = "object" }
  $spec = @{
    flowId = "phase3-real-human-tasks"
    name = "Phase 3 Human Tasks"
    description = "Verify approval, assignment, claim, idempotent action and output review"
    owner = "opentopia-e2e"
    categories = @("runtime-verification")
    source = @{ kind = "natural_language"; description = "deterministic human task verification" }
    inputSchema = $schema
    outputSchema = $schema
    graph = @{
      schemaVersion = 1
      entryNodeId = "approve"
      nodes = @(
        @{ id = "approve"; label = "Approve release"; kind = "approval"; config = @{}; inputSchema = $schema; outputSchema = $schema },
        @{ id = "output"; label = "Reviewed output"; kind = "output"; config = @{}; inputSchema = $schema; outputSchema = $schema }
      )
      edges = @(
        @{ from = "approve"; to = "output"; allowedFields = @(); dataClassification = "internal" }
      )
    }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 8; maxToolCalls = 1; maxDurationSeconds = 60; maxLoopIterations = 2 }
    riskClass = "low"
    pendingDecisions = @()
  }
  $draft = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $spec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) {
    throw "Flow validation failed"
  }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/simulate" @{
    input = @{ verificationId = $verificationId }
  }
  if ($trial.status -ne "passed") {
    throw "Flow simulation failed: $($trial | ConvertTo-Json -Depth 20 -Compress)"
  }
  $testStarted = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/test-run" @{
    input = @{ verificationId = "$verificationId-test-run" }
    startedBy = "opentopia-e2e"
  }
  $testPaused = Wait-ForRunStatus $testStarted.id @("waiting_approval", "failed", "cancelled")
  if ($testPaused.status -ne "waiting_approval") {
    throw "Flow Test Run did not pause for approval"
  }
  $testTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($testStarted.id)")
  if ($testTasks.Count -ne 1 -or $testTasks[0].taskType -ne "approval") {
    throw "Flow Test Run did not create one approval HumanTask"
  }
  Invoke-TopiaApi POST "/api/human-tasks/$($testTasks[0].id)/resolve" @{
    expectedRevision = $testTasks[0].revision
    action = "approve"
    note = "approve real Test Run before publish"
    idempotencyKey = "phase3-test-run-$verificationId"
  } | Out-Null
  $testCompleted = Wait-ForRunStatus $testStarted.id @("succeeded", "failed", "cancelled")
  if ($testCompleted.status -ne "succeeded") {
    throw "Flow Test Run failed before publish"
  }
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/publish" @{ publishedBy = "opentopia-e2e" }
  $deployment = Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $definition.flowId
    flowVersion = $definition.version
    name = "Phase 3 background deployment"
    environment = "e2e"
    createdBy = "opentopia-e2e"
    outputReviewPolicy = "always_review_output"
  }
  $started = Invoke-TopiaApi POST "/api/threads/$($thread.id)/workflow-deployments/$($deployment.id)/runs" @{
    input = @{ verificationId = $verificationId }
  }

  $approvalRun = Wait-ForRunStatus $started.id @("waiting_approval")
  $approvalTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($started.id)")
  if ($approvalTasks.Count -ne 1 -or $approvalTasks[0].taskType -ne "approval") {
    throw "Expected one approval HumanTask"
  }
  $approvalTask = Invoke-TopiaApi POST "/api/human-tasks/$($approvalTasks[0].id)/assign" @{
    expectedRevision = $approvalTasks[0].revision
    assignee = "local_operator"
  }
  $approvalTask = Invoke-TopiaApi POST "/api/human-tasks/$($approvalTask.id)/claim" @{
    expectedRevision = $approvalTask.revision
  }
  $approvalKey = "phase3-approval-$verificationId"
  $approvalResolution = Invoke-TopiaApi POST "/api/human-tasks/$($approvalTask.id)/resolve" @{
    expectedRevision = $approvalTask.revision
    action = "approve"
    note = "verified approval context"
    idempotencyKey = $approvalKey
  }
  $approvalDuplicate = Invoke-TopiaApi POST "/api/human-tasks/$($approvalTask.id)/resolve" @{
    expectedRevision = $approvalTask.revision
    action = "approve"
    note = "verified approval context"
    idempotencyKey = $approvalKey
  }
  if ($approvalDuplicate.task.resolution.commandId -ne $approvalResolution.task.resolution.commandId) {
    throw "Duplicate approval did not return the same command identity"
  }

  $reviewRun = Wait-ForRunStatus $started.id @("waiting_human", "failed", "cancelled")
  if ($reviewRun.status -ne "waiting_human") {
    throw "Production Flow did not pause for output review: $($reviewRun.status)"
  }
  $reviewTasks = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($started.id)")
  if ($reviewTasks.Count -ne 1 -or $reviewTasks[0].taskType -ne "output_review") {
    throw "Expected one output review HumanTask"
  }
  $reviewTask = Invoke-TopiaApi POST "/api/human-tasks/$($reviewTasks[0].id)/claim" @{
    expectedRevision = $reviewTasks[0].revision
  }
  $reviewKey = "phase3-output-review-$verificationId"
  $reviewResolution = Invoke-TopiaApi POST "/api/human-tasks/$($reviewTask.id)/resolve" @{
    expectedRevision = $reviewTask.revision
    action = "approve"
    note = "output matches the expected checkpoint"
    idempotencyKey = $reviewKey
  }
  $reviewDuplicate = Invoke-TopiaApi POST "/api/human-tasks/$($reviewTask.id)/resolve" @{
    expectedRevision = $reviewTask.revision
    action = "approve"
    note = "output matches the expected checkpoint"
    idempotencyKey = $reviewKey
  }
  if ($reviewDuplicate.task.resolution.commandId -ne $reviewResolution.task.resolution.commandId) {
    throw "Duplicate output review did not return the same command identity"
  }

  $completed = Wait-ForRunStatus $started.id @("succeeded", "failed", "cancelled")
  if ($completed.status -ne "succeeded" -or -not $completed.outputReviewed) {
    throw "Reviewed Flow Run did not succeed"
  }
  # Windows PowerShell 5.1 keeps an empty Invoke-RestMethod JSON array as one
  # pipeline object. Assign first so @() observes its zero elements correctly.
  $pendingResponse = Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($started.id)"
  $pending = @($pendingResponse)
  if ($pending.Count -ne 0) {
    throw "Resolved Run still has pending HumanTasks: $($pending | ConvertTo-Json -Depth 20 -Compress)"
  }

  $result = [ordered]@{
    verifiedAt = [DateTime]::UtcNow.ToString("o")
    service = "opentopia-server"
    transport = "background-http-api"
    threadId = $thread.id
    deploymentId = $deployment.id
    runId = $completed.id
    testRunId = $testCompleted.id
    status = $completed.status
    outputReviewRequired = $completed.outputReviewRequired
    outputReviewed = $completed.outputReviewed
    approvalTaskId = $approvalTask.id
    approvalRevision = $approvalResolution.task.revision
    outputReviewTaskId = $reviewTask.id
    outputReviewRevision = $reviewResolution.task.revision
    checkpointCount = @($completed.checkpointHistory).Count
    pendingHumanTasks = $pending.Count
  }
  $resultPath = Join-Path $ArtifactRoot "result.json"
  $result | ConvertTo-Json -Depth 20 | Set-Content -LiteralPath $resultPath -Encoding UTF8
  $result | ConvertTo-Json -Depth 20
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id
    Wait-Process -Id $server.Id -Timeout 10 -ErrorAction SilentlyContinue
  }
  foreach ($entry in $environment.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $previousEnvironment[$entry.Key], "Process")
  }
}
