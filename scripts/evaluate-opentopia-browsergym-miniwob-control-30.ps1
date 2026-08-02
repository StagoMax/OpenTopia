param(
  [ValidateRange(1, 6)][int]$Workers = 1,
  [ValidateRange(30000, 300000)][int]$TimeoutMs = 120000,
  [string]$OutputDirectory = "",
  [ValidateSet("all", "first", "none")][string]$ProviderPreflight = "none",
  [ValidateRange(0, 300)][int]$InterTaskDelaySeconds = 10,
  [ValidateRange(0, 5)][int]$ProviderRateLimitRetries = 3,
  [ValidateRange(15, 600)][int]$ProviderRateLimitBackoffSeconds = 90,
  [switch]$AllowScreenshots,
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Get-FailureMode {
  param([Parameter(Mandatory = $true)][object]$Task)

  if ($Task.status -eq "passed") { return $null }
  if ($Task.timedOut -and -not $Task.diagnostics.timing.firstModelOutputAt) { return "model_no_output_timeout" }
  if ($Task.timedOut) { return "turn_timeout" }
  if ($Task.providerRateLimited) { return "provider_rate_limited" }
  if ($Task.status -eq "infra_error") { return "infrastructure" }
  if ($Task.diagnostics.handoffs.Count -gt 0) { return "manual_handoff" }
  if ($Task.diagnostics.browserActions -eq 0) { return "no_browser_action" }
  if ($Task.diagnostics.failedBrowserActions.Count -gt 0) { return "browser_action_error" }
  return "task_not_completed"
}

function Read-JsonFileWithRetry {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [ValidateRange(1, 20)][int]$Attempts = 8
  )

  $lastError = $null
  for ($attempt = 1; $attempt -le $Attempts; $attempt += 1) {
    if (Test-Path -LiteralPath $Path -PathType Leaf) {
      try {
        return [IO.File]::ReadAllText($Path) | ConvertFrom-Json
      } catch {
        $lastError = $_.Exception.Message
      }
    }
    if ($attempt -lt $Attempts) { Start-Sleep -Milliseconds 250 }
  }
  return [PSCustomObject]@{ parseError = $lastError }
}

$tasks = @(
  "miniwob.click-button",
  "miniwob.click-button-sequence",
  "miniwob.click-checkboxes",
  "miniwob.click-checkboxes-large",
  "miniwob.click-checkboxes-soft",
  "miniwob.click-checkboxes-transfer",
  "miniwob.click-collapsible",
  "miniwob.click-collapsible-2",
  "miniwob.click-collapsible-2-nodelay",
  "miniwob.click-collapsible-nodelay",
  "miniwob.click-color",
  "miniwob.click-dialog",
  "miniwob.click-dialog-2",
  "miniwob.click-link",
  "miniwob.click-menu",
  "miniwob.click-menu-2",
  "miniwob.click-option",
  "miniwob.click-scroll-list",
  "miniwob.click-shades",
  "miniwob.click-shape",
  "miniwob.click-tab",
  "miniwob.click-tab-2",
  "miniwob.click-tab-2-easy",
  "miniwob.click-tab-2-hard",
  "miniwob.click-tab-2-medium",
  "miniwob.click-test",
  "miniwob.click-test-2",
  "miniwob.click-test-transfer",
  "miniwob.click-widget",
  "miniwob.number-checkboxes"
)

$repoRoot = Split-Path -Parent $PSScriptRoot
$runnerPath = Join-Path $PSScriptRoot "evaluate-opentopia-browsergym-miniwob.ps1"
if (-not (Test-Path -LiteralPath $runnerPath -PathType Leaf)) {
  throw "BrowserGym MiniWoB runner was not found: $runnerPath"
}
$runId = "browsergym-miniwob-control-30-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ") + "-" + [Guid]::NewGuid().ToString("N").Substring(0, 8)
$runRoot = if ($OutputDirectory) {
  if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory } else { Join-Path $repoRoot $OutputDirectory }
} else {
  Join-Path $repoRoot ".opentopia\evaluations\$runId"
}
New-Item -ItemType Directory -Path $runRoot -Force | Out-Null

$workerCount = [Math]::Min($Workers, $tasks.Count)
$groups = @()
for ($index = 0; $index -lt $workerCount; $index += 1) {
  $groups += ,([System.Collections.Generic.List[string]]::new())
}
for ($index = 0; $index -lt $tasks.Count; $index += 1) {
  $groups[$index % $workerCount].Add($tasks[$index])
}

$workerRuns = @()
for ($index = 0; $index -lt $groups.Count; $index += 1) {
  $workerRoot = Join-Path $runRoot ("worker-{0:D2}" -f ($index + 1))
  $stdout = Join-Path $runRoot ("worker-{0:D2}.stdout.log" -f ($index + 1))
  $stderr = Join-Path $runRoot ("worker-{0:D2}.stderr.log" -f ($index + 1))
  $taskArgument = @($groups[$index]) -join ","
  $arguments = @(
    "-NoProfile",
    "-ExecutionPolicy", "Bypass",
    "-File", $runnerPath,
    "-TaskList", $taskArgument,
    "-TimeoutMs", $TimeoutMs,
    "-OutputDirectory", $workerRoot,
    "-ProviderPreflight", $ProviderPreflight,
    "-InterTaskDelaySeconds", $InterTaskDelaySeconds,
    "-ProviderRateLimitRetries", $ProviderRateLimitRetries,
    "-ProviderRateLimitBackoffSeconds", $ProviderRateLimitBackoffSeconds
  )
  if ($SkipBuild) {
    $arguments += "-SkipBuild"
  }
  if ($AllowScreenshots) {
    $arguments += "-AllowScreenshots"
  }
  $process = Start-Process `
    -FilePath powershell.exe `
    -ArgumentList $arguments `
    -WorkingDirectory $repoRoot `
    -RedirectStandardOutput $stdout `
    -RedirectStandardError $stderr `
    -PassThru `
    -WindowStyle Hidden
  $workerRuns += [PSCustomObject]@{
    index = $index + 1
    tasks = @($groups[$index])
    process = $process
    root = $workerRoot
    stdout = $stdout
    stderr = $stderr
  }
}

foreach ($worker in $workerRuns) {
  $maximumMs = ($worker.tasks.Count * (
    $TimeoutMs +
    30000 +
    ($InterTaskDelaySeconds * 1000) +
    (($TimeoutMs + 30000 + ($ProviderRateLimitBackoffSeconds * 1000)) * $ProviderRateLimitRetries)
  )) + 60000
  if (-not $worker.process.WaitForExit($maximumMs)) {
    Stop-Process -Id $worker.process.Id -Force
  }
}

$taskResults = @()
$workerResults = @()
foreach ($worker in $workerRuns) {
  $summaryPath = Join-Path $worker.root "summary.json"
  $summary = $null
  $summaryError = $null
  if (Test-Path -LiteralPath $summaryPath -PathType Leaf) {
    $candidate = Read-JsonFileWithRetry $summaryPath
    if ($candidate.parseError) {
      $summaryError = $candidate.parseError
    } else {
      $summary = $candidate
      $taskResults += @($summary.tasks)
    }
  }
  $reportedTasks = @($summary.tasks | ForEach-Object { $_.task })
  foreach ($task in $worker.tasks | Where-Object { $_ -notin $reportedTasks }) {
    $taskResults += [ordered]@{
      task = $task
    status = "infra_error"
    providerRateLimited = $false
    timedOut = $false
      browsergym = $null
      diagnostics = [ordered]@{ browserActions = 0; failedBrowserActions = @(); handoffs = @(); applicationErrors = @() }
      error = if ($summaryError) { "Worker summary could not be parsed: $summaryError" } else { "Worker did not report a result" }
    }
  }
  $worker.process.Refresh()
  $workerResults += [ordered]@{
    worker = $worker.index
    tasks = $worker.tasks
    exited = $worker.process.HasExited
    exitCode = if ($worker.process.HasExited) { $worker.process.ExitCode } else { $null }
    summary = if ($summary) { $summaryPath.Substring($repoRoot.Length).TrimStart('\', '/') } else { $null }
    summaryError = $summaryError
  }
}

$failures = @($taskResults | Where-Object { $_.status -ne "passed" } | ForEach-Object {
  [ordered]@{
    task = $_.task
    status = $_.status
    mode = Get-FailureMode $_
    reward = $_.browsergym.reward
    completed = $_.browsergym.completed
    error = $_.error
    failedBrowserActions = $_.diagnostics.failedBrowserActions
    handoffs = $_.diagnostics.handoffs
    applicationErrors = $_.diagnostics.applicationErrors
  }
})
$summary = [ordered]@{
  benchmark = "BrowserGym MiniWoB++ DOM Control 30"
  benchmarkVersion = "browsergym-miniwob 0.14.3"
  miniwobCommit = "7fd85d71a4b60325c6585396ec4f48377d049838"
  runId = $runId
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  configuration = [ordered]@{
    textOnly = -not $AllowScreenshots
    workers = $workerCount
    timeoutMs = $TimeoutMs
    taskCount = $tasks.Count
    providerPreflight = $ProviderPreflight
    interTaskDelaySeconds = $InterTaskDelaySeconds
    providerRateLimitRetries = $ProviderRateLimitRetries
    providerRateLimitBackoffSeconds = $ProviderRateLimitBackoffSeconds
  }
  workers = $workerResults
  aggregate = [ordered]@{
    total = $tasks.Count
    reported = $taskResults.Count
    passed = @($taskResults | Where-Object { $_.status -eq "passed" }).Count
    failed = @($taskResults | Where-Object { $_.status -eq "failed" }).Count
    infraErrors = @($taskResults | Where-Object { $_.status -eq "infra_error" }).Count
    providerRateLimited = @($taskResults | Where-Object { $_.providerRateLimited }).Count
    timedOut = @($taskResults | Where-Object { $_.timedOut }).Count
    modelNoOutputTimeouts = @($taskResults | Where-Object {
      $_.timedOut -and -not $_.diagnostics.timing.firstModelOutputAt
    }).Count
  }
  tasks = $taskResults
  failures = $failures
}
$summaryPath = Join-Path $runRoot "summary.json"
[IO.File]::WriteAllText($summaryPath, (($summary | ConvertTo-Json -Depth 30) + "`n"), [Text.UTF8Encoding]::new($false))

$reportLines = @(
  "# BrowserGym MiniWoB++ DOM Control 30",
  "",
  "Text-only: $($summary.configuration.textOnly)",
  "",
  "| Task | Status | Reward | Failure mode |",
  "| --- | --- | ---: | --- |"
)
foreach ($task in $taskResults | Sort-Object task) {
  $mode = Get-FailureMode $task
  $reward = if ($null -eq $task.browsergym.reward) { "" } else { $task.browsergym.reward }
  $reportLines += "| $($task.task) | $($task.status) | $reward | $mode |"
}
if ($taskResults.Count -lt $tasks.Count) {
  $reportLines += "| missing results | infra_error |  | worker did not emit a summary |"
}
[IO.File]::WriteAllText((Join-Path $runRoot "report.md"), (($reportLines -join "`n") + "`n"), [Text.UTF8Encoding]::new($false))
$summary | ConvertTo-Json -Depth 30
