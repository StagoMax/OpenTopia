[CmdletBinding()]
param(
  [string]$ServerPath = "",
  [int]$Port = 8895,
  [int]$SinkPort = 8896,
  [string]$ArtifactRoot = ""
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$verificationId = "flow-automation-{0}" -f ([guid]::NewGuid().ToString("N"))
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
$sinkOutputPath = Join-Path $ArtifactRoot "webhook-deliveries.jsonl"
$sinkStdoutPath = Join-Path $ArtifactRoot "sink.stdout.log"
$sinkStderrPath = Join-Path $ArtifactRoot "sink.stderr.log"
$apiToken = "opentopia-automation-e2e-token-0123456789abcdef0123456789"
$triggerToken = "workflow-trigger-e2e-token-0123456789"
$outputToken = "workflow-output-e2e-token-0123456789"
$baseUrl = "http://127.0.0.1:$Port"
$sinkUrl = "http://127.0.0.1:$SinkPort"
$headers = @{ Authorization = "Bearer $apiToken" }
$environment = @{
  OPENTOPIA_API_TOKEN = $apiToken
  OPENTOPIA_PLUGIN_HOME = (Join-Path $ArtifactRoot "plugins")
  OPENTOPIA_BUNDLED_PLUGIN_HOME = (Join-Path $ArtifactRoot "bundled-plugins")
  OPENTOPIA_RUNTIME_HOME = (Join-Path $ArtifactRoot "runtime")
  OPENTOPIA_ARTIFACTS_DIR = (Join-Path $ArtifactRoot "artifacts")
  OPENTOPIA_ENTERPRISE_ENABLED = "true"
  OPENTOPIA_OFFICE_RUNTIME_AUTO_INSTALL = "false"
  OPENTOPIA_WORKFLOW_AUTOMATION_INTERVAL_MS = "250"
  OPENTOPIA_WORKFLOW_WEBHOOK_RATE_LIMIT_PER_MINUTE = "2"
  WORKFLOW_TRIGGER_TOKEN = $triggerToken
  WORKFLOW_OUTPUT_TOKEN = $outputToken
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

function Invoke-ExpectedError {
  param(
    [Parameter(Mandatory = $true)][string]$Uri,
    [Parameter(Mandatory = $true)][int]$StatusCode,
    [hashtable]$RequestHeaders = @{},
    [object]$Body = @{}
  )
  try {
    Invoke-RestMethod -Method Post -Uri $Uri -Headers $RequestHeaders -ContentType "application/json" -Body ($Body | ConvertTo-Json -Depth 40 -Compress) -TimeoutSec 15 | Out-Null
  } catch {
    $actual = [int]$_.Exception.Response.StatusCode
    if ($actual -eq $StatusCode) { return }
    throw "Expected HTTP $StatusCode from $Uri, received $actual"
  }
  throw "Expected HTTP $StatusCode from $Uri, but request succeeded"
}

function Expand-TopiaItems {
  param([object]$Value)
  if ($null -eq $Value) { return }
  if ($Value -is [System.Array]) {
    foreach ($item in $Value) { Expand-TopiaItems $item }
    return
  }
  if ($Value.PSObject.Properties.Name -contains "value" -and $Value.PSObject.Properties.Name -contains "Count" -and $Value.value -is [System.Array]) {
    foreach ($item in $Value.value) { Expand-TopiaItems $item }
    return
  }
  Write-Output $Value
}

function Wait-ForEndpoint {
  param([string]$Uri, [Diagnostics.Process]$Process, [string]$Name)
  $deadline = [DateTime]::UtcNow.AddSeconds(90)
  do {
    if ($Process.HasExited) { throw "$Name exited before becoming healthy" }
    try {
      $response = Invoke-RestMethod -Method Get -Uri $Uri -Headers $headers -TimeoutSec 2
      if ($response.ok) { return }
    } catch { Start-Sleep -Milliseconds 200 }
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "$Name did not become healthy"
}

function Wait-ForRunStatus {
  param([string]$RunId, [string[]]$Statuses)
  $deadline = [DateTime]::UtcNow.AddSeconds(60)
  do {
    $run = Invoke-TopiaApi GET "/api/flow-runs/$RunId"
    if ($run.status -in $Statuses) { return $run }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Flow Run $RunId did not reach $($Statuses -join ', '): $($run | ConvertTo-Json -Depth 20 -Compress)"
}

function Complete-ProductionRun {
  param([string]$RunId, [string]$KeyPrefix)
  for ($step = 0; $step -lt 4; $step++) {
    $run = Wait-ForRunStatus $RunId @("waiting_approval", "waiting_human", "succeeded", "failed", "cancelled")
    if ($run.status -eq "succeeded") { return $run }
    if ($run.status -in @("failed", "cancelled")) { throw "Run $RunId ended as $($run.status): $($run.error)" }
    $tasks = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$RunId"))
    if ($tasks.Count -ne 1) { throw "Expected one Flow HumanTask for $RunId, received $($tasks.Count)" }
    $task = $tasks[0]
    $action = if ($task.taskType -in @("approval", "output_review")) { "approve" } else { throw "Unexpected Flow HumanTask $($task.taskType)" }
    Invoke-TopiaApi POST "/api/human-tasks/$($task.id)/resolve" @{
      expectedRevision = $task.revision
      action = $action
      idempotencyKey = "$KeyPrefix-$($task.taskType)"
      note = "P5 background verification"
    } | Out-Null
  }
  throw "Run $RunId did not finish within the HumanTask step budget"
}

function Wait-ForDelivery {
  param([string]$DeploymentId, [string[]]$Statuses)
  $deadline = [DateTime]::UtcNow.AddSeconds(30)
  do {
    $receipts = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/workflow-delivery-receipts?deploymentId=$DeploymentId"))
    $receipt = $receipts | Select-Object -First 1
    if ($receipt -and $receipt.status -in $Statuses) { return $receipt }
    Start-Sleep -Milliseconds 100
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "DeliveryReceipt for $DeploymentId did not reach $($Statuses -join ', ')"
}

function New-Deployment {
  param([object]$Definition, [string]$Name, [object]$Output)
  Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $Definition.flowId
    flowVersion = $Definition.version
    name = $Name
    environment = "e2e"
    createdBy = "opentopia-e2e"
    output = $Output
  }
}

$server = $null
$sink = $null
try {
  $nodePath = (Get-Command node -ErrorAction Stop).Source
  $sink = Start-Process -FilePath $nodePath -ArgumentList @((Join-Path $PSScriptRoot "workflow-webhook-sink.mjs"), $SinkPort, $sinkOutputPath) -WorkingDirectory $repoRoot -RedirectStandardOutput $sinkStdoutPath -RedirectStandardError $sinkStderrPath -PassThru -WindowStyle Hidden
  Wait-ForEndpoint "$sinkUrl/health" $sink "Webhook sink"

  $server = Start-Process -FilePath $ServerPath -ArgumentList @("--host", "127.0.0.1", "--port", $Port, "--db", $databasePath, "--permission", "full-access") -WorkingDirectory $repoRoot -RedirectStandardOutput $stdoutPath -RedirectStandardError $stderrPath -PassThru -WindowStyle Hidden
  Wait-ForEndpoint "$baseUrl/health" $server "OpenTopia server"

  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "P5 background automation verification"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }
  $schema = @{ type = "object" }
  $spec = @{
    flowId = "phase5-automation-$verificationId"
    name = "Phase 5 Automation"
    description = "Deterministic external trigger and delivery verification"
    owner = "opentopia-e2e"
    categories = @("runtime-verification")
    source = @{ kind = "natural_language"; description = "P5 real automation verification" }
    inputSchema = $schema
    outputSchema = $schema
    graph = @{
      schemaVersion = 1
      entryNodeId = "approve"
      nodes = @(
        @{ id = "approve"; label = "Approve automation"; kind = "approval"; config = @{}; inputSchema = $schema; outputSchema = $schema },
        @{ id = "output"; label = "Automation output"; kind = "output"; config = @{}; inputSchema = $schema; outputSchema = $schema }
      )
      edges = @(@{ from = "approve"; to = "output"; allowedFields = @(); dataClassification = "internal" })
    }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 8; maxToolCalls = 1; maxDurationSeconds = 60; maxLoopIterations = 2 }
    riskClass = "low"
    pendingDecisions = @()
  }
  $draft = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $spec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) { throw "P5 Flow validation failed" }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/simulate" @{ input = @{ orderId = "trial" } }
  if ($trial.status -ne "passed") { throw "P5 Flow simulation failed" }
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/publish" @{ publishedBy = "opentopia-e2e" }

  $webhookDeployment = New-Deployment $definition "P5 webhook primary" @{ kind = "webhook"; endpoint = "$sinkUrl/deliver"; credentialRef = "env:WORKFLOW_OUTPUT_TOKEN" }
  $canaryDeployment = New-Deployment $definition "P5 canary inbox" @{ kind = "inbox" }
  $humanDeployment = New-Deployment $definition "P5 human handoff" @{ kind = "human_task"; title = "Confirm business handoff"; description = "Acknowledge downstream business processing"; assignedTo = "local_operator" }
  $retryDeployment = New-Deployment $definition "P5 retry delivery" @{ kind = "webhook"; endpoint = "$sinkUrl/fail-once"; credentialRef = "env:WORKFLOW_OUTPUT_TOKEN" }

  $webhookTriggerId = [guid]::NewGuid().ToString()
  $release = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "orders-webhook-$verificationId"
    environment = "e2e"
    threadId = $thread.id
    deploymentId = $webhookDeployment.id
    trigger = @{ kind = "webhook"; triggerId = $webhookTriggerId; tokenRef = "env:WORKFLOW_TRIGGER_TOKEN" }
    createdBy = "opentopia-e2e"
  }
  $hookUri = "$baseUrl/hooks/workflows/$webhookTriggerId"
  Invoke-ExpectedError $hookUri 403 @{ "x-opentopia-trigger-token" = "wrong"; "idempotency-key" = "wrong-token" } @{ orderId = "A" }
  $publicHeaders = @{ "x-opentopia-trigger-token" = $triggerToken; "idempotency-key" = "order-A" }
  $first = Invoke-RestMethod -Method Post -Uri $hookUri -Headers $publicHeaders -ContentType "application/json" -Body (@{ orderId = "A" } | ConvertTo-Json -Compress)
  $duplicate = Invoke-RestMethod -Method Post -Uri $hookUri -Headers $publicHeaders -ContentType "application/json" -Body (@{ orderId = "A" } | ConvertTo-Json -Compress)
  if (-not $duplicate.reused -or $duplicate.run.id -ne $first.run.id -or $duplicate.invocation.id -ne $first.invocation.id) { throw "Webhook idempotency did not reuse the same invocation and run" }
  Invoke-ExpectedError $hookUri 409 $publicHeaders @{ orderId = "different" }
  $secondHeaders = @{ "x-opentopia-trigger-token" = $triggerToken; "idempotency-key" = "order-B" }
  Invoke-RestMethod -Method Post -Uri $hookUri -Headers $secondHeaders -ContentType "application/json" -Body (@{ orderId = "B" } | ConvertTo-Json -Compress) | Out-Null
  $thirdHeaders = @{ "x-opentopia-trigger-token" = $triggerToken; "idempotency-key" = "order-C" }
  Invoke-ExpectedError $hookUri 429 $thirdHeaders @{ orderId = "C" }

  $completedWebhookRun = Complete-ProductionRun $first.run.id "webhook-$verificationId"
  $webhookReceipt = Wait-ForDelivery $webhookDeployment.id @("delivered")
  $sinkRecords = @(Get-Content -LiteralPath $sinkOutputPath | Where-Object { $_ } | ForEach-Object { $_ | ConvertFrom-Json })
  $deliveredRecord = $sinkRecords | Where-Object { $_.url -eq "/deliver" -and $_.body.runId -eq $first.run.id } | Select-Object -First 1
  if (-not $deliveredRecord -or $deliveredRecord.authorization -ne "Bearer $outputToken" -or $deliveredRecord.idempotencyKey -ne $webhookReceipt.idempotencyKey) { throw "Webhook output did not carry the credential reference and stable delivery key" }

  $evaluationBody = @{ runId = $first.run.id; evaluator = "p5-e2e"; score = 0.95; passed = $true; labels = @("delivery", "quality"); note = "real background verification" }
  $evaluation = Invoke-TopiaApi POST "/api/workflow-evaluations" $evaluationBody
  $evaluationDuplicate = Invoke-TopiaApi POST "/api/workflow-evaluations" $evaluationBody
  if ($evaluation.id -ne $evaluationDuplicate.id) { throw "Evaluation idempotency failed" }
  $summary = Invoke-TopiaApi GET "/api/workflow-evaluation-summary?deploymentId=$($webhookDeployment.id)"
  if ($summary.evaluationCount -ne 1 -or $summary.passRate -ne 1 -or $summary.deliveryStatusCounts.delivered -lt 1) { throw "Evaluation summary did not aggregate run and delivery data" }

  $release = Invoke-TopiaApi POST "/api/workflow-releases/$($release.id)/canary" @{ expectedRevision = $release.revision; deploymentId = $canaryDeployment.id; percent = 25 }
  if ($release.canaryDeploymentId -ne $canaryDeployment.id -or $release.canaryPercent -ne 25) { throw "Canary was not configured" }
  $release = Invoke-TopiaApi POST "/api/workflow-releases/$($release.id)/promote" @{ expectedRevision = $release.revision }
  if ($release.primaryDeploymentId -ne $canaryDeployment.id -or $release.previousPrimaryDeploymentId -ne $webhookDeployment.id) { throw "Canary promotion did not preserve rollback target" }
  $release = Invoke-TopiaApi POST "/api/workflow-releases/$($release.id)/rollback" @{ expectedRevision = $release.revision }
  if ($release.primaryDeploymentId -ne $webhookDeployment.id) { throw "Rollback did not restore the previous primary" }

  $eventRelease = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "crm-event-$verificationId"; environment = "e2e"; threadId = $thread.id; deploymentId = $canaryDeployment.id
    trigger = @{ kind = "event_subscription"; triggerId = [guid]::NewGuid().ToString(); source = "crm"; eventType = "record.updated" }
    createdBy = "opentopia-e2e"
  }
  $eventResults = @(Expand-TopiaItems (Invoke-TopiaApi POST "/api/workflow-events" @{ source = "crm"; eventType = "record.updated"; idempotencyKey = "crm-1"; payload = @{ recordId = "r-1" } }))
  $eventDuplicate = @(Expand-TopiaItems (Invoke-TopiaApi POST "/api/workflow-events" @{ source = "crm"; eventType = "record.updated"; idempotencyKey = "crm-1"; payload = @{ recordId = "r-1" } }))
  if ($eventResults.Count -ne 1 -or -not $eventDuplicate[0].reused -or $eventResults[0].run.id -ne $eventDuplicate[0].run.id) { throw "Event Subscription did not preserve idempotency" }

  $scheduleRelease = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "schedule-$verificationId"; environment = "e2e"; threadId = $thread.id; deploymentId = $canaryDeployment.id
    trigger = @{ kind = "schedule"; triggerId = [guid]::NewGuid().ToString(); intervalSeconds = 60; nextFireAt = [DateTime]::UtcNow.AddSeconds(-1).ToString("o") }
    createdBy = "opentopia-e2e"
  }
  $scheduleDeadline = [DateTime]::UtcNow.AddSeconds(15)
  do {
    $scheduleInvocations = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/workflow-trigger-invocations?releaseId=$($scheduleRelease.id)"))
    if ($scheduleInvocations.Count -gt 0) { break }
    Start-Sleep -Milliseconds 200
  } while ([DateTime]::UtcNow -lt $scheduleDeadline)
  if ($scheduleInvocations.Count -ne 1 -or -not $scheduleInvocations[0].id) { throw "Schedule did not create exactly one due invocation" }

  $humanRelease = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "human-$verificationId"; environment = "e2e"; threadId = $thread.id; deploymentId = $humanDeployment.id
    trigger = @{ kind = "event_subscription"; triggerId = [guid]::NewGuid().ToString(); source = "internal"; eventType = "human.test" }
    createdBy = "opentopia-e2e"
  }
  $humanInvocation = Invoke-TopiaApi POST "/api/workflow-releases/$($humanRelease.id)/invoke" @{ idempotencyKey = "human-1"; input = @{ caseId = "h-1" } }
  Complete-ProductionRun $humanInvocation.run.id "human-$verificationId" | Out-Null
  $humanReceipt = Wait-ForDelivery $humanDeployment.id @("waiting_human")
  $deliveryTasks = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/human-tasks?status=pending&threadId=$($thread.id)&kind=manual"))
  if ($deliveryTasks.Count -ne 1 -or $deliveryTasks[0].sourceKind -ne "delivery_receipt" -or $deliveryTasks[0].sourceId -ne $humanReceipt.id) { throw "Non-Agent delivery HumanTask did not enter the unified Inbox" }
  $humanResolution = Invoke-TopiaApi POST "/api/human-tasks/$($deliveryTasks[0].id)/resolve" @{ expectedRevision = $deliveryTasks[0].revision; action = "acknowledge"; idempotencyKey = "human-ack-$verificationId" }
  if ($humanResolution.deliveryReceipt.status -ne "delivered" -or $humanResolution.run) { throw "HumanTask acknowledgment did not resolve the DeliveryReceipt source" }

  $retryRelease = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "retry-$verificationId"; environment = "e2e"; threadId = $thread.id; deploymentId = $retryDeployment.id
    trigger = @{ kind = "event_subscription"; triggerId = [guid]::NewGuid().ToString(); source = "internal"; eventType = "retry.test" }
    createdBy = "opentopia-e2e"
  }
  $retryInvocation = Invoke-TopiaApi POST "/api/workflow-releases/$($retryRelease.id)/invoke" @{ idempotencyKey = "retry-1"; input = @{ caseId = "r-1" } }
  Complete-ProductionRun $retryInvocation.run.id "retry-$verificationId" | Out-Null
  $failedReceipt = Wait-ForDelivery $retryDeployment.id @("failed")
  $recoveryTasks = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/human-tasks?status=pending&threadId=$($thread.id)&kind=recovery"))
  if ($recoveryTasks.Count -ne 1 -or $recoveryTasks[0].taskType -ne "recovery") { throw "Failed delivery did not create a Recovery HumanTask" }
  $retryResolution = Invoke-TopiaApi POST "/api/human-tasks/$($recoveryTasks[0].id)/resolve" @{ expectedRevision = $recoveryTasks[0].revision; action = "retry"; idempotencyKey = "delivery-retry-$verificationId" }
  if ($retryResolution.deliveryReceipt.status -ne "delivered") { throw "Explicit DeliveryReceipt retry did not succeed" }

  $result = [ordered]@{
    verifiedAt = [DateTime]::UtcNow.ToString("o")
    service = "opentopia-server"
    transport = "background-http-api"
    foregroundAutomation = $false
    threadId = $thread.id
    workflowDefinition = "$($definition.flowId)@$($definition.version)"
    publicWebhookAuthenticated = $true
    webhookIdempotent = $true
    webhookRateLimited = $true
    webhookRunId = $completedWebhookRun.id
    webhookReceiptId = $webhookReceipt.id
    evaluationId = $evaluation.id
    evaluationPassRate = $summary.passRate
    canaryPromotedAndRolledBack = $true
    eventInvocationId = $eventResults[0].invocation.id
    scheduleInvocationId = $scheduleInvocations[0].id
    humanDeliveryTaskId = $deliveryTasks[0].id
    recoveryTaskId = $recoveryTasks[0].id
    recoveredDeliveryStatus = $retryResolution.deliveryReceipt.status
    schemaVersion = 29
  }
  $resultPath = Join-Path $ArtifactRoot "result.json"
  $result | ConvertTo-Json -Depth 40 | Set-Content -LiteralPath $resultPath -Encoding UTF8
  $result | ConvertTo-Json -Depth 40
} finally {
  foreach ($process in @($server, $sink)) {
    if ($process -and -not $process.HasExited) {
      Stop-Process -Id $process.Id
      Wait-Process -Id $process.Id -Timeout 10 -ErrorAction SilentlyContinue
    }
  }
  foreach ($entry in $environment.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $previousEnvironment[$entry.Key], "Process")
  }
}
