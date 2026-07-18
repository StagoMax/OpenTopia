param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "glm-5.2",
  [int]$Port = 8812,
  [int]$TurnTimeoutSeconds = 420,
  [string]$SummaryPath = "",
  [string]$TaskManifest = "scripts\fixtures\long-horizon\task.json",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) {
      return
    }
    $parts = $line.Split("=", 2)
    $value = $parts[1].Trim()
    if (
      $value.Length -ge 2 -and
      (($value.StartsWith('"') -and $value.EndsWith('"')) -or
        ($value.StartsWith("'") -and $value.EndsWith("'")))
    ) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    $values[$parts[0].Trim()] = $value
  }
  return $values
}

function Protect-Text {
  param(
    [AllowNull()][string]$Text,
    [Parameter(Mandatory = $true)][string]$Secret
  )

  if ([string]::IsNullOrWhiteSpace($Text)) {
    return $null
  }
  $safe = $Text.Replace($Secret, "<redacted>")
  $safe = $safe -replace '(?i)Bearer\s+[A-Za-z0-9._~+/=-]+', 'Bearer <redacted>'
  if ($safe.Length -gt 800) {
    $safe = $safe.Substring(0, 800)
  }
  return $safe
}

function Invoke-EvalApi {
  param(
    [Parameter(Mandatory = $true)][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [AllowNull()][object]$Body = $null,
    [int]$TimeoutSeconds = 20
  )

  $parameters = @{
    Method = $Method
    Uri = "http://127.0.0.1:$Port$Path"
    Headers = $script:ApiHeaders
    TimeoutSec = $TimeoutSeconds
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json"
    $parameters.Body = $Body | ConvertTo-Json -Depth 30 -Compress
  }
  return Invoke-RestMethod @parameters
}

function Expand-EvalItems {
  param([AllowNull()][object]$Value)

  if ($null -eq $Value) {
    return
  }
  if ($Value -is [System.Array]) {
    foreach ($item in $Value) {
      Expand-EvalItems $item
    }
    return
  }
  Write-Output $Value
}

function Start-EvalServer {
  param(
    [Parameter(Mandatory = $true)][string]$Label,
    [Parameter(Mandatory = $true)][string]$ServerPath,
    [Parameter(Mandatory = $true)][string]$DatabasePath,
    [Parameter(Mandatory = $true)][string]$RunRoot
  )

  $stdoutPath = Join-Path $RunRoot "server-$Label.stdout.log"
  $stderrPath = Join-Path $RunRoot "server-$Label.stderr.log"
  $process = Start-Process `
    -FilePath $ServerPath `
    -ArgumentList @(
      "--port", $Port,
      "--db", $DatabasePath,
      "--permission", "full-access"
    ) `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru `
    -WindowStyle Hidden

  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    if ($process.HasExited) {
      throw "Server exited before becoming healthy"
    }
    Start-Sleep -Milliseconds 250
    try {
      $health = Invoke-EvalApi "Get" "/health"
      if ($health.ok -and $health.service -eq "opentopia-server") {
        return $process
      }
    } catch {
    }
  }
  throw "Server did not become healthy within 30 seconds"
}

function Stop-EvalServer {
  param([AllowNull()][System.Diagnostics.Process]$Process)

  if ($Process -and -not $Process.HasExited) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(5000) | Out-Null
  }
}

function Wait-EvalTurn {
  param(
    [Parameter(Mandatory = $true)][string]$ThreadId,
    [Parameter(Mandatory = $true)][string]$UserMessageId,
    [Parameter(Mandatory = $true)][int]$TimeoutSeconds
  )

  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  $unexpectedExecutionCheckpoints = 0
  $deniedApprovals = 0
  $lastTurn = $null
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    $lastTurn = Invoke-EvalApi "Get" "/api/threads/$ThreadId/turn"
    if (-not $lastTurn -or $lastTurn.userMessageId -ne $UserMessageId) {
      continue
    }
    if ($lastTurn.status -eq "waiting_approval") {
      $pending = @(Expand-EvalItems (
        Invoke-EvalApi "Get" "/api/threads/$ThreadId/approvals?status=pending"
      ))
      if ($pending.Count -ne 1) {
        throw "Turn reached an ambiguous approval boundary"
      }
      $approvalId = [string]$pending[0].approvalId
      if (-not $approvalId) {
        throw "Pending approval did not include approvalId"
      }
      $isUnexpectedExecutionCheckpoint =
        $pending[0].action -eq "Continue agent execution"
      Invoke-EvalApi `
        -Method "Post" `
        -Path "/api/threads/$ThreadId/approvals/$approvalId/decision" `
        -Body @{ approved = $false } `
        -TimeoutSeconds $TimeoutSeconds | Out-Null
      if ($isUnexpectedExecutionCheckpoint) {
        $unexpectedExecutionCheckpoints += 1
      } else {
        $deniedApprovals += 1
      }
      continue
    }
    if ($lastTurn.status -in @("succeeded", "failed", "cancelled", "interrupted")) {
      return [PSCustomObject]@{
        Turn = $lastTurn
        UnexpectedExecutionCheckpoints = $unexpectedExecutionCheckpoints
        DeniedApprovals = $deniedApprovals
      }
    }
  }

  try {
    Invoke-EvalApi "Post" "/api/threads/$ThreadId/turn/cancel" @{} | Out-Null
  } catch {
  }
  throw "Turn exceeded the configured hard timeout"
}

function Invoke-HiddenGrader {
  param(
    [Parameter(Mandatory = $true)][string]$Workspace,
    [Parameter(Mandatory = $true)][string]$Phase,
    [Parameter(Mandatory = $true)][string]$GraderPath
  )

  $output = & node $GraderPath $Workspace $Phase 2>&1
  $exitCode = $LASTEXITCODE
  $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
  try {
    $result = $text | ConvertFrom-Json
  } catch {
    return [PSCustomObject]@{
      phase = $Phase
      passed = $false
      passedChecks = 0
      totalChecks = 1
      checks = @([PSCustomObject]@{
        id = "grader-output"
        passed = $false
        detail = "grader did not return JSON (exit $exitCode)"
      })
    }
  }
  return $result
}

function Get-TrajectoryMetrics {
  param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Events)

  $Events = @(Expand-EvalItems $Events)
  $toolStarts = @($Events | Where-Object { $_.payload.type -eq "tool_call_started" })
  $toolFinishes = @($Events | Where-Object { $_.payload.type -eq "tool_call_finished" })
  $planEvents = @($Events | Where-Object { $_.payload.type -eq "plan_updated" })
  $usageEvents = @($Events | Where-Object { $_.payload.type -eq "token_usage" })
  $unexpectedExecutionCheckpoints = @($Events | Where-Object {
    $_.payload.type -eq "approval_requested" -and
    $_.payload.action -eq "Continue agent execution"
  })
  $toolByName = [ordered]@{}
  foreach ($event in $toolStarts) {
    $name = [string]$event.payload.call.name
    if (-not $toolByName.Contains($name)) {
      $toolByName[$name] = 0
    }
    $toolByName[$name] += 1
  }
  $testToolCalls = @($toolStarts | Where-Object {
    $_.payload.call.name -eq "shell" -and
    [string]$_.payload.call.input.command -match '(?i)(npm\s+test|node\s+--test)'
  }).Count
  $completionToolCalls = @($toolStarts | Where-Object {
    $_.payload.call.name -eq "complete_task"
  }).Count
  $blockedLoopGuardToolCalls = @($toolFinishes | Where-Object {
    $_.payload.result.metadata.loopGuardBlocked -eq $true
  }).Count
  $blockedCompletionModeToolCalls = @($toolFinishes | Where-Object {
    $_.payload.result.metadata.completionModeBlocked -eq $true
  }).Count
  $verifiedPlanCompletionCalls = @($toolFinishes | Where-Object {
    $_.payload.result.metadata.toolName -eq "update_plan" -and
    $_.payload.result.metadata.success -eq $true -and
    (
      $_.payload.result.metadata.currentScopeComplete -eq $true -or
      $_.payload.result.metadata.allStepsComplete -eq $true
    )
  }).Count
  $fallbackVerifiedCompletions = @($planEvents | Where-Object {
    [string]$_.payload.plan.explanation -like "Runtime fallback reconciled the durable plan*"
  }).Count
  $latestPlan = if ($planEvents.Count -gt 0) {
    $planEvents[$planEvents.Count - 1].payload.plan
  } else {
    $null
  }
  $planStatus = [ordered]@{ pending = 0; inProgress = 0; completed = 0 }
  if ($latestPlan) {
    foreach ($step in @($latestPlan.steps)) {
      switch ([string]$step.status) {
        "pending" { $planStatus.pending += 1 }
        "in_progress" { $planStatus.inProgress += 1 }
        "completed" { $planStatus.completed += 1 }
      }
    }
  }
  $inputTokens = [int64](($usageEvents | ForEach-Object {
    $_.payload.input_tokens
  } | Measure-Object -Sum).Sum)
  $outputTokens = [int64](($usageEvents | ForEach-Object {
    $_.payload.output_tokens
  } | Measure-Object -Sum).Sum)
  $totalTokens = [int64](($usageEvents | ForEach-Object {
    $_.payload.total_tokens
  } | Measure-Object -Sum).Sum)
  return [ordered]@{
    eventCount = $Events.Count
    turnCount = @($Events | ForEach-Object { $_.turnId } | Where-Object { $_ } |
      Select-Object -Unique).Count
    toolCallsStarted = $toolStarts.Count
    toolCallsFinished = $toolFinishes.Count
    toolCallsByName = $toolByName
    planUpdates = $planEvents.Count
    unexpectedExecutionCheckpoints = $unexpectedExecutionCheckpoints.Count
    latestPlan = $planStatus
    testToolCalls = $testToolCalls
    completionToolCalls = $completionToolCalls
    verifiedPlanCompletionCalls = $verifiedPlanCompletionCalls
    fallbackVerifiedCompletions = $fallbackVerifiedCompletions
    blockedLoopGuardToolCalls = $blockedLoopGuardToolCalls
    blockedCompletionModeToolCalls = $blockedCompletionModeToolCalls
    inputTokens = $inputTokens
    outputTokens = $outputTokens
    totalTokens = $totalTokens
    errorEvents = @($Events | Where-Object { $_.payload.type -eq "error" }).Count
  }
}

function Test-FilesForSecret {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string]$Secret
  )

  $secretBytes = [Text.Encoding]::UTF8.GetBytes($Secret)
  foreach ($file in Get-ChildItem -LiteralPath $Root -File -Recurse) {
    $bytes = [IO.File]::ReadAllBytes($file.FullName)
    if ($bytes.Length -lt $secretBytes.Length) {
      continue
    }
    for ($offset = 0; $offset -le $bytes.Length - $secretBytes.Length; $offset += 1) {
      $match = $true
      for ($index = 0; $index -lt $secretBytes.Length; $index += 1) {
        if ($bytes[$offset + $index] -ne $secretBytes[$index]) {
          $match = $false
          break
        }
      }
      if ($match) {
        return $false
      }
    }
  }
  return $true
}

$repoRoot = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\dev-env.ps1"

$taskManifestPath = if ([IO.Path]::IsPathRooted($TaskManifest)) {
  $TaskManifest
} else {
  Join-Path $repoRoot $TaskManifest
}
$taskManifestPath = (Resolve-Path -LiteralPath $taskManifestPath).Path
$taskRoot = Split-Path -Parent $taskManifestPath
$task = Get-Content -Raw -Encoding UTF8 -LiteralPath $taskManifestPath | ConvertFrom-Json
foreach ($required in @(
  "id",
  "title",
  "seedDirectory",
  "graderPath",
  "publicTestFile",
  "phase1",
  "phase2"
)) {
  if (-not $task.$required) {
    throw "Task manifest is missing required field: $required"
  }
}
if (-not $task.phase1.prompt -or -not $task.phase1.graderPhase) {
  throw "Task manifest phase1 is incomplete"
}
if (-not $task.phase2.prompt -or -not $task.phase2.graderPhase) {
  throw "Task manifest phase2 is incomplete"
}
$seed = Join-Path $taskRoot ([string]$task.seedDirectory)
$graderPath = Join-Path $taskRoot ([string]$task.graderPath)
if (-not (Test-Path -LiteralPath $seed -PathType Container)) {
  throw "Task seed directory was not found: $seed"
}
if (-not (Test-Path -LiteralPath $graderPath -PathType Leaf)) {
  throw "Task grader was not found: $graderPath"
}

$values = ConvertFrom-DotEnvFile $EnvFile
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "The selected provider profile is incomplete"
}
if ($ExpectedModel -and $model -ne $ExpectedModel) {
  throw "Selected model does not match the expected model"
}

$savedEnvironment = @{}
foreach ($name in @(
  "OPENTOPIA_API_KEY",
  "OPENTOPIA_OPENAI_BASE_URL",
  "OPENTOPIA_MODEL",
  "OPENTOPIA_DB",
  "OPENTOPIA_SANDBOX_MODE",
  "OPENTOPIA_SANDBOX_ENFORCEMENT",
  "OPENTOPIA_SANDBOX_NETWORK"
)) {
  $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}
[Environment]::SetEnvironmentVariable("OPENTOPIA_API_KEY", $apiKey, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_OPENAI_BASE_URL", $baseUrl, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_MODEL", $model, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_MODE", "workspace-write", "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_ENFORCEMENT", "best-effort", "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_NETWORK", "deny", "Process")

$script:ApiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }
$modelSlug = ($model.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')
$taskSlug = (([string]$task.id).ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')
$runId = "$modelSlug-$taskSlug-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$runRoot = Join-Path $repoRoot ".opentopia\evaluations\$runId"
$workspace = Join-Path $runRoot "workspace"
$database = Join-Path $runRoot "evaluation.db"
$probePath = Join-Path $runRoot "provider-probe.json"
$trajectoryPhase1Path = Join-Path $runRoot "trajectory-phase1.json"
$trajectoryFinalPath = Join-Path $runRoot "trajectory-final.json"
$resultPath = Join-Path $runRoot "result.json"
$server = $null
$startedAt = Get-Date
$phase1StartedAt = $null
$phase2StartedAt = $null
$phase1ElapsedMs = $null
$phase2ElapsedMs = $null
$restartElapsedMs = $null
$providerProbe = $null
$providerHealth = $null
$phase1Turn = $null
$phase2Turn = $null
$phase1UnexpectedExecutionCheckpoints = 0
$phase2UnexpectedExecutionCheckpoints = 0
$phase1DeniedApprovals = 0
$phase2DeniedApprovals = 0
$thread = $null
$eventsPhase1 = @()
$eventsFinal = @()
$recovery = [ordered]@{
  serverRestarted = $false
  threadRecovered = $false
  turnRecovered = $false
  messagesRecovered = $false
  eventsRecovered = $false
  durablePlanRecovered = $false
  activePlanRecovered = $false
}
$runError = $null

New-Item -ItemType Directory -Path $workspace -Force | Out-Null
Copy-Item -Path (Join-Path $seed "*") -Destination $workspace -Recurse -Force

& git -C $workspace init --quiet
& git -C $workspace config user.name "OpenTopia Eval"
& git -C $workspace config user.email "eval@localhost"
& git -C $workspace add .
& git -C $workspace commit --quiet -m "evaluation baseline"
if ($LASTEXITCODE -ne 0) {
  throw "Failed to initialize the evaluation fixture"
}

$baselinePublicOutput = & node --test (Join-Path $workspace ([string]$task.publicTestFile)) 2>&1
$baselinePublicExit = $LASTEXITCODE
$baselineLibrary = Invoke-HiddenGrader `
  $workspace `
  ([string]$task.phase1.graderPhase) `
  $graderPath
$baselineFull = Invoke-HiddenGrader `
  $workspace `
  ([string]$task.phase2.graderPhase) `
  $graderPath

try {
  $probeText = & "$PSScriptRoot\probe-openai-compatible.ps1" `
    -EnvFile $EnvFile `
    -Profile $Profile `
    -ExpectedModel $ExpectedModel `
    -OutputPath $probePath
  $providerProbe = (($probeText | ForEach-Object { $_.ToString() }) -join "`n") |
    ConvertFrom-Json
  if (-not $providerProbe.compatibleWithOpenTopia) {
    throw "Provider compatibility probe failed"
  }

  $targetDir = Join-Path $repoRoot ".opentopia\verify-target"
  $env:CARGO_TARGET_DIR = $targetDir
  if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
      cargo build -p opentopia-server
      if ($LASTEXITCODE -ne 0) {
        throw "opentopia-server build failed"
      }
    } finally {
      Pop-Location
    }
  }
  $serverPath = Join-Path $targetDir "debug\opentopia-server.exe"
  if (-not (Test-Path -LiteralPath $serverPath)) {
    $serverPath = Join-Path $targetDir "debug\opentopia-server"
  }
  if (-not (Test-Path -LiteralPath $serverPath)) {
    throw "opentopia-server debug binary was not found"
  }

  $server = Start-EvalServer "phase1" $serverPath $database $runRoot
  $providerHealth = Invoke-EvalApi "Post" "/api/provider/test" @{}
  if (-not $providerHealth.reachable -or -not $providerHealth.modelAvailable) {
    throw "OpenTopia provider health check failed"
  }
  $settings = Invoke-EvalApi "Get" "/api/settings"
  $activeProvider = @($settings.providers | Where-Object {
    $_.id -eq $settings.activeProviderId
  })[0]
  if (-not $activeProvider) {
    $activeProvider = @($settings.providers)[0]
  }
  if (
    $activeProvider.kind -ne "open_ai_compatible" -or
    $activeProvider.model -ne $ExpectedModel -or
    $activeProvider.baseUrl.TrimEnd("/") -ne $baseUrl
  ) {
    throw "OpenTopia active provider settings do not match the selected profile"
  }

  $thread = Invoke-EvalApi "Post" "/api/threads" @{
    title = [string]$task.title
    workspaceRoot = $workspace
  }
  $phase1Prompt = [string]$task.phase1.prompt
  $phase1StartedAt = Get-Date
  $phase1Message = Invoke-EvalApi "Post" "/api/threads/$($thread.id)/messages" @{
    content = $phase1Prompt
  }
  $phase1Wait = Wait-EvalTurn $thread.id $phase1Message.id $TurnTimeoutSeconds
  $phase1ElapsedMs = [int64]((Get-Date) - $phase1StartedAt).TotalMilliseconds
  $phase1Turn = $phase1Wait.Turn
  $phase1UnexpectedExecutionCheckpoints = $phase1Wait.UnexpectedExecutionCheckpoints
  $phase1DeniedApprovals = $phase1Wait.DeniedApprovals
  $eventsPhase1 = @(Expand-EvalItems (
    Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"
  ))
  $trajectoryPhase1 = Invoke-EvalApi "Get" "/api/threads/$($thread.id)/trajectory"
  $trajectoryPhase1Json = $trajectoryPhase1 | ConvertTo-Json -Depth 100
  if ($trajectoryPhase1Json.Contains($apiKey)) {
    throw "Secret audit rejected phase-1 trajectory"
  }
  [IO.File]::WriteAllText(
    $trajectoryPhase1Path,
    "$trajectoryPhase1Json`n",
    [Text.UTF8Encoding]::new($false)
  )

  Stop-EvalServer $server
  $server = $null
  $restartStartedAt = Get-Date
  $server = Start-EvalServer "phase2" $serverPath $database $runRoot
  $restartElapsedMs = [int64]((Get-Date) - $restartStartedAt).TotalMilliseconds
  $recovery.serverRestarted = $true
  $threadsRecovered = @(Expand-EvalItems (Invoke-EvalApi "Get" "/api/threads"))
  $recovery.threadRecovered = @($threadsRecovered | Where-Object {
    $_.id -eq $thread.id
  }).Count -eq 1
  $recoveredTurn = Invoke-EvalApi "Get" "/api/threads/$($thread.id)/turn"
  $recovery.turnRecovered =
    $recoveredTurn.userMessageId -eq $phase1Message.id -and
    $recoveredTurn.status -eq $phase1Turn.status
  $messagesRecovered = @(Expand-EvalItems (
    Invoke-EvalApi "Get" "/api/threads/$($thread.id)/messages"
  ))
  $recovery.messagesRecovered = $messagesRecovered.Count -ge 2
  $recoveredEvents = @(Expand-EvalItems (
    Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"
  ))
  $recovery.eventsRecovered = $recoveredEvents.Count -eq $eventsPhase1.Count
  $recoveredPlans = @($recoveredEvents | Where-Object {
    $_.payload.type -eq "plan_updated"
  })
  $recovery.durablePlanRecovered = $recoveredPlans.Count -gt 0
  if ($recoveredPlans.Count -gt 0) {
    $recoveredPlan = $recoveredPlans[$recoveredPlans.Count - 1].payload.plan
    $recovery.activePlanRecovered = @($recoveredPlan.steps | Where-Object {
      $_.status -ne "completed"
    }).Count -gt 0
  }

  $phase2Prompt = [string]$task.phase2.prompt
  $phase2StartedAt = Get-Date
  $phase2Message = Invoke-EvalApi "Post" "/api/threads/$($thread.id)/messages" @{
    content = $phase2Prompt
  }
  $phase2Wait = Wait-EvalTurn $thread.id $phase2Message.id $TurnTimeoutSeconds
  $phase2ElapsedMs = [int64]((Get-Date) - $phase2StartedAt).TotalMilliseconds
  $phase2Turn = $phase2Wait.Turn
  $phase2UnexpectedExecutionCheckpoints = $phase2Wait.UnexpectedExecutionCheckpoints
  $phase2DeniedApprovals = $phase2Wait.DeniedApprovals
  $eventsFinal = @(Expand-EvalItems (
    Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"
  ))
  $trajectoryFinal = Invoke-EvalApi "Get" "/api/threads/$($thread.id)/trajectory"
  $trajectoryFinalJson = $trajectoryFinal | ConvertTo-Json -Depth 100
  if ($trajectoryFinalJson.Contains($apiKey)) {
    throw "Secret audit rejected final trajectory"
  }
  [IO.File]::WriteAllText(
    $trajectoryFinalPath,
    "$trajectoryFinalJson`n",
    [Text.UTF8Encoding]::new($false)
  )
} catch {
  $runError = Protect-Text $_.Exception.Message $apiKey
  if ($thread -and $server -and -not $server.HasExited) {
    try {
      $eventsFinal = @(Expand-EvalItems (
        Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"
      ))
      $failedTrajectory = Invoke-EvalApi `
        "Get" `
        "/api/threads/$($thread.id)/trajectory"
      $failedTrajectoryJson = $failedTrajectory | ConvertTo-Json -Depth 100
      if (-not $failedTrajectoryJson.Contains($apiKey)) {
        [IO.File]::WriteAllText(
          $trajectoryFinalPath,
          "$failedTrajectoryJson`n",
          [Text.UTF8Encoding]::new($false)
        )
      }
    } catch {
    }
  }
} finally {
  Stop-EvalServer $server
  foreach ($name in $savedEnvironment.Keys) {
    [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
  }
}

$phase1Grade = Invoke-HiddenGrader `
  $workspace `
  ([string]$task.phase1.graderPhase) `
  $graderPath
$finalGrade = Invoke-HiddenGrader `
  $workspace `
  ([string]$task.phase2.graderPhase) `
  $graderPath
if ($eventsFinal.Count -eq 0 -and $eventsPhase1.Count -gt 0) {
  $eventsFinal = $eventsPhase1
}
$phase1Metrics = Get-TrajectoryMetrics $eventsPhase1
$metrics = Get-TrajectoryMetrics $eventsFinal
$minPlanUpdates = if ($null -ne $task.process.minPlanUpdates) {
  [int]$task.process.minPlanUpdates
} else { 2 }
$minTestToolCalls = if ($null -ne $task.process.minTestToolCalls) {
  [int]$task.process.minTestToolCalls
} else { 2 }
$minCompletedPlanSteps = if ($null -ne $task.process.minCompletedPlanSteps) {
  [int]$task.process.minCompletedPlanSteps
} else { 4 }
$requireExplicitCompletion = $task.process.requireExplicitCompletion -eq $true
$phase1CompletionSignals =
  $phase1Metrics.completionToolCalls +
  $phase1Metrics.verifiedPlanCompletionCalls +
  $phase1Metrics.fallbackVerifiedCompletions
$completionSignals =
  $metrics.completionToolCalls +
  $metrics.verifiedPlanCompletionCalls +
  $metrics.fallbackVerifiedCompletions
$latestPlanComplete =
  $metrics.planUpdates -gt 0 -and
  $metrics.latestPlan.pending -eq 0 -and
  $metrics.latestPlan.inProgress -eq 0 -and
  $metrics.latestPlan.completed -ge $minCompletedPlanSteps
$recoveryPassed =
  $recovery.serverRestarted -and
  $recovery.threadRecovered -and
  $recovery.turnRecovered -and
  $recovery.messagesRecovered -and
  $recovery.eventsRecovered -and
  $recovery.durablePlanRecovered -and
  $recovery.activePlanRecovered
$processContractPassed =
  $metrics.planUpdates -ge $minPlanUpdates -and
  $metrics.testToolCalls -ge $minTestToolCalls -and
  $latestPlanComplete -and
  (
    -not $requireExplicitCompletion -or
    (
      $phase1CompletionSignals -ge 1 -and
      $completionSignals -ge 2
    )
  )
$providerPassed = $null -ne $providerProbe -and $providerProbe.compatibleWithOpenTopia
$turnsPassed =
  $null -ne $phase1Turn -and
  $null -ne $phase2Turn -and
  $phase1Turn.status -eq "succeeded" -and
  $phase2Turn.status -eq "succeeded"
$secretAuditPassed = Test-FilesForSecret $runRoot $apiKey
$overallPassed =
  $providerPassed -and
  $turnsPassed -and
  $phase1Grade.passed -and
  $finalGrade.passed -and
  $recoveryPassed -and
  $processContractPassed -and
  $secretAuditPassed -and
  -not $runError

$result = [ordered]@{
  schemaVersion = 1
  runId = $runId
  startedAt = $startedAt.ToUniversalTime().ToString("o")
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  status = if ($overallPassed) { "passed" } else { "failed" }
  objectiveScoringOnly = $true
  task = [ordered]@{
    id = [string]$task.id
    title = [string]$task.title
    manifest = $taskManifestPath.Substring($repoRoot.Length).TrimStart('\', '/')
    phase1Grader = [string]$task.phase1.graderPhase
    phase2Grader = [string]$task.phase2.graderPhase
  }
  provider = [ordered]@{
    profile = $Profile
    baseUrl = $baseUrl
    model = $model
    credentials = "redacted:set"
    compatibleWithOpenTopia = $providerPassed
    modelListed = if ($providerProbe) { $providerProbe.models.modelListed } else { $false }
    streamChat = if ($providerProbe) { $providerProbe.streamChat.status } else { $null }
    streamToolsAuto = if ($providerProbe) { $providerProbe.streamToolsAuto.status } else { $null }
    streamToolContinuation = if ($providerProbe) { $providerProbe.streamToolContinuation.status } else { $null }
    streamSerializedToolHistory = if ($providerProbe) { $providerProbe.streamSerializedToolHistory.status } else { $null }
    streamCompactedToolHistory = if ($providerProbe) { $providerProbe.streamCompactedToolHistory.status } else { $null }
    streamToolsForced = if ($providerProbe) { $providerProbe.streamToolsForced.status } else { $null }
    openTopiaHealthReachable = if ($providerHealth) { $providerHealth.reachable } else { $false }
    openTopiaModelAvailable = if ($providerHealth) { $providerHealth.modelAvailable } else { $false }
  }
  timing = [ordered]@{
    totalMs = [int64]((Get-Date) - $startedAt).TotalMilliseconds
    phase1Ms = $phase1ElapsedMs
    restartMs = $restartElapsedMs
    phase2Ms = $phase2ElapsedMs
    hardTimeoutPerTurnSeconds = $TurnTimeoutSeconds
  }
  baseline = [ordered]@{
    publicTestsExitCode = $baselinePublicExit
    library = $baselineLibrary
    full = $baselineFull
    expectedFailureObserved =
      $baselinePublicExit -ne 0 -and
      -not $baselineLibrary.passed -and
      -not $baselineFull.passed
  }
  turns = @(
    [ordered]@{
      phase = 1
      status = if ($phase1Turn) { $phase1Turn.status } else { "not_completed" }
      elapsedMs = $phase1ElapsedMs
      unexpectedExecutionCheckpoints = $phase1UnexpectedExecutionCheckpoints
      deniedApprovals = $phase1DeniedApprovals
    },
    [ordered]@{
      phase = 2
      status = if ($phase2Turn) { $phase2Turn.status } else { "not_completed" }
      elapsedMs = $phase2ElapsedMs
      unexpectedExecutionCheckpoints = $phase2UnexpectedExecutionCheckpoints
      deniedApprovals = $phase2DeniedApprovals
    }
  )
  recovery = $recovery
  recoveryPassed = $recoveryPassed
  trajectoryMetrics = $metrics
  processContract = [ordered]@{
    passed = $processContractPassed
    requireExplicitCompletion = $requireExplicitCompletion
    phase1CompletionCalls = $phase1Metrics.completionToolCalls
    totalCompletionCalls = $metrics.completionToolCalls
    phase1VerifiedPlanCompletions = $phase1Metrics.verifiedPlanCompletionCalls
    totalVerifiedPlanCompletions = $metrics.verifiedPlanCompletionCalls
    phase1FallbackVerifiedCompletions = $phase1Metrics.fallbackVerifiedCompletions
    totalFallbackVerifiedCompletions = $metrics.fallbackVerifiedCompletions
    minPlanUpdates = $minPlanUpdates
    minTestToolCalls = $minTestToolCalls
    minCompletedPlanSteps = $minCompletedPlanSteps
  }
  processContractPassed = $processContractPassed
  grading = [ordered]@{
    phase1Library = $phase1Grade
    final = $finalGrade
  }
  secretAuditPassed = $secretAuditPassed
  error = $runError
  artifacts = [ordered]@{
    runDirectory = ".opentopia/evaluations/$runId"
    providerProbe = ".opentopia/evaluations/$runId/provider-probe.json"
    phase1Trajectory = if (Test-Path $trajectoryPhase1Path) {
      ".opentopia/evaluations/$runId/trajectory-phase1.json"
    } else { $null }
    finalTrajectory = if (Test-Path $trajectoryFinalPath) {
      ".opentopia/evaluations/$runId/trajectory-final.json"
    } else { $null }
  }
}

$resultJson = $result | ConvertTo-Json -Depth 40
if ($resultJson.Contains($apiKey)) {
  throw "Secret audit rejected final evaluation report"
}
[IO.File]::WriteAllText($resultPath, "$resultJson`n", [Text.UTF8Encoding]::new($false))
if ($SummaryPath) {
  $summaryParent = Split-Path -Parent $SummaryPath
  if ($summaryParent) {
    New-Item -ItemType Directory -Path $summaryParent -Force | Out-Null
  }
  [IO.File]::WriteAllText($SummaryPath, "$resultJson`n", [Text.UTF8Encoding]::new($false))
}
$resultJson
if (-not $overallPassed) {
  exit 1
}
