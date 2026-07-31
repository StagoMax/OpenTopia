param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "glm-5.2",
  [int]$Port = 8842,
  [string]$SummaryPath = "",
  [string]$TaskManifest = "scripts\fixtures\computer-use\profile-editor\task.json",
  [Parameter(Mandatory = $true)][switch]$IsolatedDesktop,
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) { return }
    $parts = $line.Split("=", 2)
    $value = $parts[1].Trim()
    if ($value.Length -ge 2 -and (($value.StartsWith('"') -and $value.EndsWith('"')) -or ($value.StartsWith("'") -and $value.EndsWith("'")))) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    $values[$parts[0].Trim()] = $value
  }
  return $values
}

function Invoke-EvalApi {
  param(
    [Parameter(Mandatory = $true)][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [AllowNull()][object]$Body = $null
  )

  $parameters = @{
    Method = $Method
    Uri = "http://127.0.0.1:$Port$Path"
    Headers = $script:ApiHeaders
    TimeoutSec = 20
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json"
    $parameters.Body = $Body | ConvertTo-Json -Depth 30 -Compress
  }
  Invoke-RestMethod @parameters
}

function Expand-EvalItems {
  param([AllowNull()][object]$Value)

  if ($null -eq $Value) { return }
  if ($Value -is [System.Array]) {
    foreach ($item in $Value) { Expand-EvalItems $item }
    return
  }
  Write-Output $Value
}

function Start-EvalServer {
  param(
    [Parameter(Mandatory = $true)][string]$ServerPath,
    [Parameter(Mandatory = $true)][string]$DatabasePath,
    [Parameter(Mandatory = $true)][string]$RunRoot
  )

  $process = Start-Process `
    -FilePath $ServerPath `
    -ArgumentList @("--port", $Port, "--db", $DatabasePath, "--permission", "full-access") `
    -RedirectStandardOutput (Join-Path $RunRoot "server.stdout.log") `
    -RedirectStandardError (Join-Path $RunRoot "server.stderr.log") `
    -PassThru `
    -WindowStyle Hidden
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    if ($process.HasExited) { throw "Server exited before becoming healthy" }
    Start-Sleep -Milliseconds 250
    try {
      $health = Invoke-EvalApi "Get" "/health"
      if ($health.ok -and $health.service -eq "opentopia-server") { return $process }
    } catch {
    }
  }
  throw "Server did not become healthy within 30 seconds"
}

function Stop-EvalProcess {
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
    [ValidateRange(1, 1800)][int]$TimeoutSeconds,
    [Parameter(Mandatory = $true)][string]$AllowedWindowTitle
  )

  $approvals = @()
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ($true) {
    if ((Get-Date) -ge $deadline) {
      throw "Computer Use task exceeded the $TimeoutSeconds-second hard timeout"
    }
    Start-Sleep -Milliseconds 500
    $turn = Invoke-EvalApi "Get" "/api/threads/$ThreadId/turn"
    if (-not $turn -or $turn.userMessageId -ne $UserMessageId) { continue }
    if ($turn.status -eq "waiting_approval") {
      $pending = @(Expand-EvalItems (Invoke-EvalApi "Get" "/api/threads/$ThreadId/approvals?status=pending"))
      if ($pending.Count -ne 1) { throw "Turn reached an ambiguous approval boundary" }
      $approval = $pending[0]
      $action = [string]$approval.action
      $approved = $action -match '^computer:(list_windows|observe):'
      if ($action -match '^computer:(click|type|keypress|scroll|drag|wait):') {
        $approved = [string]$approval.reason -like "*$AllowedWindowTitle*"
      }
      Invoke-EvalApi "Post" "/api/threads/$ThreadId/approvals/$($approval.approvalId)/decision" @{ approved = $approved } | Out-Null
      $approvals += [PSCustomObject]@{
        action = $action
        approved = $approved
        reason = [string]$approval.reason
      }
      continue
    }
    if ($turn.status -in @("succeeded", "failed", "cancelled", "interrupted")) {
      return [PSCustomObject]@{ Turn = $turn; Approvals = $approvals }
    }
  }
}

function Invoke-FixtureGrader {
  param(
    [Parameter(Mandatory = $true)][string]$GraderPath,
    [Parameter(Mandatory = $true)][string]$StatePath,
    [Parameter(Mandatory = $true)][string]$ManifestPath
  )

  $output = & node $GraderPath $StatePath $ManifestPath 2>&1
  $exitCode = $LASTEXITCODE
  $text = ($output | ForEach-Object { $_.ToString() }) -join "`n"
  try { return $text | ConvertFrom-Json } catch {
    return [PSCustomObject]@{
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
}

function Get-ComputerMetrics {
  param([Parameter(Mandatory = $true)][AllowEmptyCollection()][object[]]$Events)

  $tools = [ordered]@{}
  $actions = @()
  [int64]$inputTokens = 0
  [int64]$outputTokens = 0
  [int64]$totalTokens = 0
  foreach ($event in @(Expand-EvalItems $Events)) {
    $payload = $event.payload
    if ($null -eq $payload) { continue }
    if ($payload.type -eq "tool_call_started") {
      $name = [string]$payload.call.name
      if (-not $tools.Contains($name)) { $tools[$name] = 0 }
      $tools[$name] += 1
      if ($name -eq "computer") { $actions += [string]$payload.call.input.action }
    }
    if ($payload.type -eq "token_usage") {
      if ($null -ne $payload.input_tokens) { $inputTokens += [int64]$payload.input_tokens }
      if ($null -ne $payload.output_tokens) { $outputTokens += [int64]$payload.output_tokens }
      if ($null -ne $payload.total_tokens) { $totalTokens += [int64]$payload.total_tokens }
    }
  }
  [ordered]@{
    toolCallsByName = $tools
    computerActions = $actions
    computerActionCount = $actions.Count
    unexpectedTools = @($tools.Keys | Where-Object { $_ -notin @("computer", "set_plan", "update_plan", "complete_task") })
    inputTokens = $inputTokens
    outputTokens = $outputTokens
    totalTokens = $totalTokens
  }
}

function Test-FilesForSecret {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string]$Secret
  )

  foreach ($file in Get-ChildItem -LiteralPath $Root -File -Recurse) {
    $content = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($file.FullName))
    if ($content.IndexOf($Secret, [StringComparison]::Ordinal) -ge 0) { return $false }
  }
  return $true
}

function Quote-PowerShellArgument {
  param([Parameter(Mandatory = $true)][string]$Value)

  '"' + $Value.Replace('"', '\"') + '"'
}

if ([Environment]::OSVersion.Platform -ne [PlatformID]::Win32NT) {
  throw "Computer Use evaluation requires an interactive Windows desktop"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\dev-env.ps1"
$taskManifestPath = if ([IO.Path]::IsPathRooted($TaskManifest)) { $TaskManifest } else { Join-Path $repoRoot $TaskManifest }
$taskManifestPath = (Resolve-Path -LiteralPath $taskManifestPath).Path
$taskRoot = Split-Path -Parent $taskManifestPath
$task = Get-Content -Raw -Encoding UTF8 -LiteralPath $taskManifestPath | ConvertFrom-Json
foreach ($required in @("id", "title", "fixtureScript", "graderPath", "fixture", "prompt", "process")) {
  if (-not $task.$required) { throw "Task manifest is missing required field: $required" }
}
$fixtureScriptPath = Join-Path $taskRoot ([string]$task.fixtureScript)
$graderPath = Join-Path $taskRoot ([string]$task.graderPath)
if (-not (Test-Path -LiteralPath $fixtureScriptPath -PathType Leaf)) { throw "Fixture script was not found: $fixtureScriptPath" }
if (-not (Test-Path -LiteralPath $graderPath -PathType Leaf)) { throw "Fixture grader was not found: $graderPath" }

$values = ConvertFrom-DotEnvFile $EnvFile
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) { throw "The selected provider profile is incomplete" }
if ($ExpectedModel -and $model -ne $ExpectedModel) { throw "Selected model does not match the expected model" }

$savedEnvironment = @{}
foreach ($name in @("OPENTOPIA_API_KEY", "OPENTOPIA_OPENAI_BASE_URL", "OPENTOPIA_MODEL", "OPENTOPIA_DB", "OPENTOPIA_SANDBOX_MODE", "OPENTOPIA_SANDBOX_ENFORCEMENT", "OPENTOPIA_SANDBOX_NETWORK")) {
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
$harnessRoot = Join-Path $runRoot "harness"
$fixtureStatePath = Join-Path $harnessRoot "fixture-state.json"
$database = Join-Path $runRoot "evaluation.db"
$probePath = Join-Path $runRoot "provider-probe.json"
$trajectoryPath = Join-Path $runRoot "trajectory.json"
$resultPath = Join-Path $runRoot "result.json"
$startedAt = Get-Date
$server = $null
$fixture = $null
$thread = $null
$turn = $null
$turnElapsedMs = $null
$turnTimeoutSeconds = $null
$events = @()
$approvals = @()
$providerProbe = $null
$providerHealth = $null
$baseline = $null
$runError = $null

New-Item -ItemType Directory -Path $workspace, $harnessRoot -Force | Out-Null
Set-Content -LiteralPath (Join-Path $workspace ".gitignore") -Value "*" -Encoding UTF8

try {
  $baseline = Invoke-FixtureGrader $graderPath $fixtureStatePath $taskManifestPath
  if ($baseline.passed) { throw "Fixture baseline unexpectedly satisfied the hidden grader" }

  # This process is outside the Agent workspace; only its visible window is in scope.
  $fixture = Start-Process `
    -FilePath "powershell.exe" `
    -ArgumentList @("-NoProfile", "-Sta", "-ExecutionPolicy", "Bypass", "-File", (Quote-PowerShellArgument $fixtureScriptPath), "-StatePath", (Quote-PowerShellArgument $fixtureStatePath)) `
    -WorkingDirectory $harnessRoot `
    -PassThru
  Start-Sleep -Milliseconds 750
  if ($fixture.HasExited) { throw "Computer Use fixture exited before the agent started" }

  $probeText = & "$PSScriptRoot\probe-openai-compatible.ps1" -EnvFile $EnvFile -Profile $Profile -ExpectedModel $ExpectedModel -OutputPath $probePath
  $providerProbe = (($probeText | ForEach-Object { $_.ToString() }) -join "`n") | ConvertFrom-Json
  if (-not $providerProbe.compatibleWithOpenTopia) { throw "Provider compatibility probe failed" }

  $targetDir = Join-Path $repoRoot ".opentopia\verify-target"
  $env:CARGO_TARGET_DIR = $targetDir
  if (-not $SkipBuild) {
    Push-Location $repoRoot
    try {
      cargo build -p opentopia-server
      if ($LASTEXITCODE -ne 0) { throw "opentopia-server build failed" }
    } finally { Pop-Location }
  }
  $serverPath = Join-Path $targetDir "debug\opentopia-server.exe"
  if (-not (Test-Path -LiteralPath $serverPath)) { $serverPath = Join-Path $targetDir "debug\opentopia-server" }
  if (-not (Test-Path -LiteralPath $serverPath)) { throw "opentopia-server debug binary was not found" }

  $server = Start-EvalServer $serverPath $database $runRoot
  $providerHealth = Invoke-EvalApi "Post" "/api/provider/test" @{}
  if (-not $providerHealth.reachable -or -not $providerHealth.modelAvailable) { throw "OpenTopia provider health check failed" }
  $settings = Invoke-EvalApi "Get" "/api/settings"
  $activeProvider = @($settings.providers | Where-Object { $_.id -eq $settings.activeProviderId })[0]
  if (-not $activeProvider) { $activeProvider = @($settings.providers)[0] }
  if ($activeProvider.kind -notin @("openai_compatible", "openai_responses") -or $activeProvider.model -ne $ExpectedModel -or $activeProvider.baseUrl.TrimEnd("/") -ne $baseUrl) {
    throw "OpenTopia active provider settings do not match the selected profile"
  }

  $thread = Invoke-EvalApi "Post" "/api/threads" @{ title = [string]$task.title; workspaceRoot = $workspace }
  $turnStartedAt = Get-Date
  $message = Invoke-EvalApi "Post" "/api/threads/$($thread.id)/messages" @{ content = [string]$task.prompt }
  $turnTimeoutSeconds = if ($null -ne $task.process.maxTurnSeconds) {
    [int]$task.process.maxTurnSeconds
  } else {
    300
  }
  $wait = Wait-EvalTurn $thread.id $message.id $turnTimeoutSeconds ([string]$task.fixture.title)
  $turn = $wait.Turn
  $approvals = $wait.Approvals
  $turnElapsedMs = [int64]((Get-Date) - $turnStartedAt).TotalMilliseconds
  $events = @(Expand-EvalItems (Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"))
  $trajectory = Invoke-EvalApi "Get" "/api/threads/$($thread.id)/trajectory"
  $trajectoryJson = $trajectory | ConvertTo-Json -Depth 100
  if ($trajectoryJson.Contains($apiKey)) { throw "Secret audit rejected trajectory" }
  [IO.File]::WriteAllText($trajectoryPath, "$trajectoryJson`n", [Text.UTF8Encoding]::new($false))
} catch {
  $safeError = $_.Exception.Message.Replace($apiKey, "<redacted>")
  $runError = if ($safeError.Length -gt 800) { $safeError.Substring(0, 800) } else { $safeError }
  if ($thread -and $server -and -not $server.HasExited) {
    try {
      $events = @(Expand-EvalItems (Invoke-EvalApi "Get" "/api/threads/$($thread.id)/events"))
      $trajectory = Invoke-EvalApi "Get" "/api/threads/$($thread.id)/trajectory"
      $trajectoryJson = $trajectory | ConvertTo-Json -Depth 100
      if (-not $trajectoryJson.Contains($apiKey)) { [IO.File]::WriteAllText($trajectoryPath, "$trajectoryJson`n", [Text.UTF8Encoding]::new($false)) }
    } catch {
    }
  }
} finally {
  Stop-EvalProcess $server
  Stop-EvalProcess $fixture
  foreach ($name in $savedEnvironment.Keys) { [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process") }
}

$finalGrade = Invoke-FixtureGrader $graderPath $fixtureStatePath $taskManifestPath
$metrics = Get-ComputerMetrics $events
$requiredActions = @($task.process.requiredComputerActions | ForEach-Object { [string]$_ })
$missingActions = @($requiredActions | Where-Object { $_ -notin $metrics.computerActions })
$lastClick = [Array]::LastIndexOf([string[]]$metrics.computerActions, "click")
$postSaveObserved = -not $task.process.requirePostSaveObservation
if (-not $postSaveObserved -and $lastClick -ge 0 -and $lastClick + 1 -lt $metrics.computerActions.Count) {
  $postSaveObserved = @($metrics.computerActions[($lastClick + 1)..($metrics.computerActions.Count - 1)] | Where-Object { $_ -eq "observe" }).Count -gt 0
}
$providerPassed = $null -ne $providerProbe -and $providerProbe.compatibleWithOpenTopia
$turnPassed = $null -ne $turn -and $turn.status -eq "succeeded"
$toolDisciplinePassed = $metrics.unexpectedTools.Count -eq 0
$computerProcessPassed = $missingActions.Count -eq 0 -and $postSaveObserved
$secretAuditPassed = Test-FilesForSecret $runRoot $apiKey
$overallPassed = $providerPassed -and $turnPassed -and $finalGrade.passed -and $toolDisciplinePassed -and $computerProcessPassed -and $secretAuditPassed -and -not $runError

$result = [ordered]@{
  schemaVersion = 1
  runId = $runId
  startedAt = $startedAt.ToUniversalTime().ToString("o")
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  status = if ($overallPassed) { "passed" } else { "failed" }
  objectiveScoringOnly = $true
  task = [ordered]@{ id = [string]$task.id; title = [string]$task.title; manifest = $taskManifestPath.Substring($repoRoot.Length).TrimStart('\', '/') }
  environment = [ordered]@{ platform = "windows"; isolatedDesktopConfirmed = $IsolatedDesktop.IsPresent; fixtureTitle = [string]$task.fixture.title }
  provider = [ordered]@{ profile = $Profile; baseUrl = $baseUrl; model = $model; credentials = "redacted:set"; compatibleWithOpenTopia = $providerPassed; healthReachable = if ($providerHealth) { $providerHealth.reachable } else { $false }; modelAvailable = if ($providerHealth) { $providerHealth.modelAvailable } else { $false } }
  timing = [ordered]@{ totalMs = [int64]((Get-Date) - $startedAt).TotalMilliseconds; turnMs = $turnElapsedMs }
  baseline = $baseline
  turn = [ordered]@{ status = if ($turn) { $turn.status } else { "not_completed" } }
  approvals = $approvals
  computer = $metrics
  process = [ordered]@{ passed = $computerProcessPassed; requiredActions = $requiredActions; missingActions = $missingActions; postSaveObserved = $postSaveObserved; toolDisciplinePassed = $toolDisciplinePassed; maxTurnSeconds = if ($turnTimeoutSeconds) { $turnTimeoutSeconds } else { $null } }
  grading = $finalGrade
  secretAuditPassed = $secretAuditPassed
  error = $runError
  artifacts = [ordered]@{ runDirectory = ".opentopia/evaluations/$runId"; trajectory = if (Test-Path -LiteralPath $trajectoryPath) { ".opentopia/evaluations/$runId/trajectory.json" } else { $null } }
}
$resultJson = $result | ConvertTo-Json -Depth 50
if ($resultJson.Contains($apiKey)) { throw "Secret audit rejected final evaluation report" }
[IO.File]::WriteAllText($resultPath, "$resultJson`n", [Text.UTF8Encoding]::new($false))
if ($SummaryPath) {
  $summaryParent = Split-Path -Parent $SummaryPath
  if ($summaryParent) { New-Item -ItemType Directory -Path $summaryParent -Force | Out-Null }
  [IO.File]::WriteAllText($SummaryPath, "$resultJson`n", [Text.UTF8Encoding]::new($false))
}
$resultJson
if (-not $overallPassed) { exit 1 }
