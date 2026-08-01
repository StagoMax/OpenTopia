param(
  [string[]]$Tasks = @("miniwob.click-test", "miniwob.click-button", "miniwob.click-button-sequence"),
  [int]$Seed = 0,
  [ValidateRange(1024, 65535)][int]$Port = 8820,
  [ValidateRange(60000, 1800000)][int]$TimeoutMs = 300000,
  [string]$OutputDirectory = "",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function New-Token {
  $bytes = [byte[]]::new(32)
  $random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
  try {
    $random.GetBytes($bytes)
  } finally {
    $random.Dispose()
  }
  return [Convert]::ToBase64String($bytes).TrimEnd("=").Replace("+", "-").Replace("/", "_")
}

function Get-FreeLoopbackPort {
  $listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, 0)
  try {
    $listener.Start()
    return ([Net.IPEndPoint]$listener.LocalEndpoint).Port
  } finally {
    $listener.Stop()
  }
}

function Stop-ChildProcess {
  param([AllowNull()][Diagnostics.Process]$Process)

  if ($Process) {
    $Process.Refresh()
  }
  if ($Process -and -not $Process.HasExited) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(5000) | Out-Null
  }
}

function Wait-Healthy {
  param(
    [Parameter(Mandatory = $true)][string]$Url,
    [Parameter(Mandatory = $true)][hashtable]$Headers,
    [Parameter(Mandatory = $true)][Diagnostics.Process]$Process,
    [Parameter(Mandatory = $true)][string]$ExpectedService,
    [int]$TimeoutSeconds = 30
  )

  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    $Process.Refresh()
    if ($Process.HasExited) {
      throw "$ExpectedService exited before becoming healthy"
    }
    try {
      $health = Invoke-RestMethod -Uri "$Url/health" -Headers $Headers -TimeoutSec 2
      if ($health.ok -and $health.service -eq $ExpectedService) {
        return
      }
    } catch {
    }
    Start-Sleep -Milliseconds 150
  }
  throw "$ExpectedService did not become healthy within $TimeoutSeconds seconds"
}

function Get-TaskEvents {
  param([Parameter(Mandatory = $true)][string]$EventsPath)

  if (-not (Test-Path -LiteralPath $EventsPath -PathType Leaf)) {
    return @()
  }
  return @([IO.File]::ReadAllLines($EventsPath) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) })
}

function Import-EvaluationEnv {
  param([Parameter(Mandatory = $true)][string]$Path)

  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#")) { return }
    if ($line.StartsWith("export ")) { $line = $line.Substring(7).Trim() }
    $separator = $line.IndexOf("=")
    if ($separator -le 0) { return }
    $name = $line.Substring(0, $separator).Trim()
    $value = $line.Substring($separator + 1).Trim()
    if ($value.Length -ge 2 -and (
      ($value.StartsWith('"') -and $value.EndsWith('"')) -or
      ($value.StartsWith("'") -and $value.EndsWith("'"))
    )) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    if ($name -and -not [Environment]::GetEnvironmentVariable($name, "Process")) {
      [Environment]::SetEnvironmentVariable($name, $value, "Process")
    }
  }
}

$repoRoot = Split-Path -Parent $PSScriptRoot
$envFile = if ($env:OPENTOPIA_ENV_FILE) { $env:OPENTOPIA_ENV_FILE } else { Join-Path $repoRoot ".env" }
if (Test-Path -LiteralPath $envFile -PathType Leaf) {
  Import-EvaluationEnv (Resolve-Path -LiteralPath $envFile).Path
}
if (-not $env:OPENTOPIA_API_KEY -and $env:OPENTOPIA_MODEL_KEY) {
  $env:OPENTOPIA_API_KEY = $env:OPENTOPIA_MODEL_KEY
}
foreach ($name in @("OPENTOPIA_API_KEY", "OPENTOPIA_OPENAI_BASE_URL", "OPENTOPIA_MODEL")) {
  if ([string]::IsNullOrWhiteSpace([Environment]::GetEnvironmentVariable($name, "Process"))) {
    throw "$name is required. Configure it in .env before running the public browser benchmark."
  }
}

$pythonPath = Join-Path $repoRoot ".opentopia\browsergym-venv\Scripts\python.exe"
$brokerPath = Join-Path $repoRoot "evaluation\adapters\browsergym_miniwob_broker.py"
$miniwobRoot = Join-Path $repoRoot ".opentopia\MiniWoB-plusplus-7fd85d71a4b60325c6585396ec4f48377d049838\miniwob\html"
$serverTarget = Join-Path $repoRoot ".opentopia\verify-target"
$chromeCandidates = @(
  "${env:ProgramFiles}\Google\Chrome\Application\chrome.exe",
  "${env:ProgramFiles(x86)}\Google\Chrome\Application\chrome.exe",
  "${env:ProgramFiles(x86)}\Microsoft\Edge\Application\msedge.exe"
)
$browserExecutable = $chromeCandidates | Where-Object { Test-Path -LiteralPath $_ -PathType Leaf } | Select-Object -First 1
foreach ($path in @($pythonPath, $brokerPath)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Required evaluation file was not found: $path"
  }
}
if (-not (Test-Path -LiteralPath $miniwobRoot -PathType Container)) {
  throw "Pinned MiniWoB++ source was not found: $miniwobRoot"
}

if (-not $SkipBuild) {
  Push-Location $repoRoot
  try {
    $env:CARGO_TARGET_DIR = $serverTarget
    cargo build -p opentopia-server
    if ($LASTEXITCODE -ne 0) {
      throw "opentopia-server build failed"
    }
  } finally {
    Pop-Location
  }
}
$serverPath = Join-Path $serverTarget "debug\opentopia-server.exe"
if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
  $serverPath = Join-Path $serverTarget "debug\opentopia-server"
}
if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
  throw "opentopia-server binary was not found; rerun without -SkipBuild"
}

$startedAt = (Get-Date).ToUniversalTime()
$runId = "browsergym-miniwob-" + $startedAt.ToString("yyyyMMddTHHmmssZ")
$runRoot = if ($OutputDirectory) {
  if ([IO.Path]::IsPathRooted($OutputDirectory)) { $OutputDirectory } else { Join-Path $repoRoot $OutputDirectory }
} else {
  Join-Path $repoRoot ".opentopia\evaluations\$runId"
}
New-Item -ItemType Directory -Path $runRoot -Force | Out-Null

$savedEnvironment = @{}
foreach ($name in @(
  "OPENTOPIA_API_TOKEN",
  "OPENTOPIA_DESKTOP_BROWSER_BROKER_URL",
  "OPENTOPIA_DESKTOP_BROWSER_BROKER_TOKEN",
  "OPENTOPIA_SANDBOX_NETWORK",
  "AGENT_EVAL_WORKSPACE",
  "AGENT_EVAL_EVENTS_PATH",
  "AGENT_EVAL_PROMPT_FILE",
  "AGENT_EVAL_TASK_ID",
  "OPENTOPIA_EVAL_BASE_URL",
  "OPENTOPIA_EVAL_TIMEOUT_MS"
)) {
  $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$apiToken = New-Token
$env:OPENTOPIA_API_TOKEN = $apiToken
$env:OPENTOPIA_SANDBOX_NETWORK = "deny"
$taskResults = @()
try {
  for ($index = 0; $index -lt $Tasks.Count; $index += 1) {
    $task = $Tasks[$index].Trim()
    if ([string]::IsNullOrWhiteSpace($task)) {
      throw "Task ids cannot be empty"
    }
    $taskRoot = Join-Path $runRoot ("{0:D2}-{1}" -f ($index + 1), ($task -replace "[^A-Za-z0-9._-]", "-"))
    New-Item -ItemType Directory -Path $taskRoot -Force | Out-Null
    $brokerToken = New-Token
    $brokerStdout = Join-Path $taskRoot "broker.stdout.log"
    $brokerStderr = Join-Path $taskRoot "broker.stderr.log"
    $brokerResultPath = Join-Path $taskRoot "browsergym-result.json"
    $serverStdout = Join-Path $taskRoot "server.stdout.log"
    $serverStderr = Join-Path $taskRoot "server.stderr.log"
    $adapterStdout = Join-Path $taskRoot "adapter.stdout.log"
    $adapterStderr = Join-Path $taskRoot "adapter.stderr.log"
    $eventsPath = Join-Path $taskRoot "events.jsonl"
    $promptPath = Join-Path $taskRoot "prompt.txt"
    $databasePath = Join-Path $taskRoot "evaluation.db"
    $brokerProcess = $null
    $serverProcess = $null
    $adapterProcess = $null
    $taskError = $null
    $brokerResult = $null
    $events = @()
    $adapterExitCode = $null
    try {
      $brokerArguments = "evaluation/adapters/browsergym_miniwob_broker.py --task $task --seed $Seed --miniwob-root `"$miniwobRoot`" --port 0 --token $brokerToken --result-path `"$brokerResultPath`""
      if ($browserExecutable) {
        $brokerArguments += " --browser-executable `"$browserExecutable`""
      }
      $brokerProcess = Start-Process `
        -FilePath $pythonPath `
        -ArgumentList $brokerArguments `
        -WorkingDirectory $repoRoot `
        -RedirectStandardOutput $brokerStdout `
        -RedirectStandardError $brokerStderr `
        -PassThru `
        -WindowStyle Hidden

      $brokerHeaders = @{ Authorization = "Bearer $brokerToken" }
      $brokerDeadline = (Get-Date).AddSeconds(30)
      $brokerUrl = $null
      while ((Get-Date) -lt $brokerDeadline) {
        $brokerProcess.Refresh()
        if ($brokerProcess.HasExited) { throw "BrowserGym broker exited before reporting its URL" }
        if (Test-Path -LiteralPath $brokerStdout -PathType Leaf) {
          try {
            $startup = Get-Content -LiteralPath $brokerStdout -Raw | ConvertFrom-Json
            if ($startup.ok -and $startup.url) {
              $brokerUrl = [string]$startup.url
              break
            }
          } catch {
          }
        }
        Start-Sleep -Milliseconds 150
      }
      if (-not $brokerUrl) { throw "BrowserGym broker did not report a URL within 30 seconds" }
      Wait-Healthy -Url $brokerUrl -Headers $brokerHeaders -Process $brokerProcess -ExpectedService "opentopia-browsergym-miniwob-broker"
      $taskInfo = Invoke-RestMethod -Uri "$brokerUrl/task" -Headers $brokerHeaders -TimeoutSec 5
      $prompt = @(
        "Complete this BrowserGym MiniWoB++ task using only the browser tool.",
        "The shared browser is already on the task page. Start with browser observe and do not navigate away.",
        "Task goal: $($taskInfo.goal)",
        "Stop after completing the task."
      ) -join "`n"
      [IO.File]::WriteAllText($promptPath, "$prompt`n", [Text.UTF8Encoding]::new($false))

      $serverPort = Get-FreeLoopbackPort
      $env:OPENTOPIA_DESKTOP_BROWSER_BROKER_URL = $brokerUrl
      $env:OPENTOPIA_DESKTOP_BROWSER_BROKER_TOKEN = $brokerToken
      $serverProcess = Start-Process `
        -FilePath $serverPath `
        -ArgumentList @("--port", $serverPort, "--db", $databasePath, "--permission", "full-access") `
        -RedirectStandardOutput $serverStdout `
        -RedirectStandardError $serverStderr `
        -PassThru `
        -WindowStyle Hidden
      $apiHeaders = @{ Authorization = "Bearer $apiToken" }
      Wait-Healthy -Url "http://127.0.0.1:$serverPort" -Headers $apiHeaders -Process $serverProcess -ExpectedService "opentopia-server"
      $providerHealth = Invoke-RestMethod -Method Post -Uri "http://127.0.0.1:$serverPort/api/provider/test" -Headers $apiHeaders -ContentType "application/json" -Body "{}" -TimeoutSec 20
      if (-not $providerHealth.reachable -or -not $providerHealth.modelAvailable) {
        throw "OpenTopia provider health check failed (reachable=$($providerHealth.reachable), modelAvailable=$($providerHealth.modelAvailable), detail=$($providerHealth.detail))"
      }

      $env:AGENT_EVAL_WORKSPACE = $repoRoot
      $env:AGENT_EVAL_EVENTS_PATH = $eventsPath
      $env:AGENT_EVAL_PROMPT_FILE = $promptPath
      $env:AGENT_EVAL_TASK_ID = $taskInfo.taskId
      $env:OPENTOPIA_EVAL_BASE_URL = "http://127.0.0.1:$serverPort"
      $env:OPENTOPIA_EVAL_TIMEOUT_MS = "$TimeoutMs"
      $adapterProcess = Start-Process `
        -FilePath (Get-Command node -ErrorAction Stop).Source `
        -ArgumentList "evaluation/adapters/opentopia-http.mjs" `
        -WorkingDirectory $repoRoot `
        -RedirectStandardOutput $adapterStdout `
        -RedirectStandardError $adapterStderr `
        -PassThru `
        -WindowStyle Hidden
      $adapterFinished = $adapterProcess.WaitForExit($TimeoutMs + 30000)
      $adapterProcess.Refresh()
      if (-not $adapterFinished -or -not $adapterProcess.HasExited) {
        throw "OpenTopia adapter exceeded the benchmark timeout"
      }
      try {
        $adapterExitCode = $adapterProcess.ExitCode
      } catch {
        $adapterExitCode = $null
      }
      if ($null -ne $adapterExitCode -and $adapterExitCode -ne 0) {
        throw "OpenTopia adapter exited with code $adapterExitCode"
      }
      $events = Get-TaskEvents $eventsPath
      if (-not ($events | Where-Object { $_.Contains('"type":"application.turn.completed"') })) {
        throw "OpenTopia adapter exited without recording a completed turn"
      }
      if ($null -eq $adapterExitCode) {
        # Some Windows Start-Process instances do not retain ExitCode after asynchronous redirection.
        $adapterExitCode = 0
      }
      $brokerResult = Invoke-RestMethod -Uri "$brokerUrl/results" -Headers $brokerHeaders -TimeoutSec 10
    } catch {
      $taskError = $_.Exception.Message.Replace($apiToken, "<redacted>").Replace($brokerToken, "<redacted>")
    } finally {
      if ($brokerUrl) {
        try {
          Invoke-RestMethod -Method Post -Uri "$brokerUrl/shutdown" -Headers $brokerHeaders -ContentType "application/json" -Body "{}" -TimeoutSec 5 | Out-Null
        } catch {
        }
      }
      Stop-ChildProcess $adapterProcess
      Stop-ChildProcess $serverProcess
      Stop-ChildProcess $brokerProcess
    }
    $browserActions = @($events | Where-Object { $_.Contains('"type":"browser.action.completed"') })
    $taskResults += [ordered]@{
      task = $task
      seed = $Seed
      status = if ($taskError) { "infra_error" } elseif ($brokerResult.success) { "passed" } else { "failed" }
      browsergym = $brokerResult
      browserActions = [ordered]@{
        total = $browserActions.Count
        succeeded = @($browserActions | Where-Object { $_.Contains('"success":true') }).Count
      }
      adapter = [ordered]@{
        exitCode = $adapterExitCode
        events = $events.Count
      }
      paths = [ordered]@{
        root = $taskRoot.Substring($repoRoot.Length).TrimStart('\', '/')
        brokerResult = if (Test-Path -LiteralPath $brokerResultPath) { $brokerResultPath.Substring($repoRoot.Length).TrimStart('\', '/') } else { $null }
      }
      error = $taskError
    }
  }
} finally {
  foreach ($name in $savedEnvironment.Keys) {
    [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
  }
}

$summary = [ordered]@{
  benchmark = "BrowserGym MiniWoB++"
  benchmarkVersion = "browsergym-miniwob 0.14.3"
  miniwobCommit = "7fd85d71a4b60325c6585396ec4f48377d049838"
  runId = $runId
  startedAt = $startedAt.ToString("o")
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  model = $env:OPENTOPIA_MODEL
  tasks = $taskResults
  aggregate = [ordered]@{
    total = $taskResults.Count
    passed = @($taskResults | Where-Object { $_.status -eq "passed" }).Count
    failed = @($taskResults | Where-Object { $_.status -eq "failed" }).Count
    infraErrors = @($taskResults | Where-Object { $_.status -eq "infra_error" }).Count
  }
}
$summaryPath = Join-Path $runRoot "summary.json"
[IO.File]::WriteAllText($summaryPath, (($summary | ConvertTo-Json -Depth 20) + "`n"), [Text.UTF8Encoding]::new($false))
$summary | ConvertTo-Json -Depth 20
if ($summary.aggregate.passed -ne $summary.aggregate.total) {
  exit 1
}
