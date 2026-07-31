param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "",
  [ValidateRange(1024, 65535)][int]$Port = 8812,
  [ValidateRange(1, 100)][int]$Repetitions = 1,
  [string]$SuitePath = "evaluation\examples\opentopia-tool-suite\suite.json",
  [string]$TargetPath = "",
  [string]$OutputDirectory = "",
  [string]$SummaryPath = "",
  [switch]$SkipBuild,
  [switch]$BrowserFixture,
  [ValidateRange(1024, 65535)][int]$BrowserFixturePort = 8999
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#")) {
      return
    }
    if ($line.StartsWith("export ")) {
      $line = $line.Substring(7).Trim()
    }
    $separator = $line.IndexOf("=")
    if ($separator -le 0) {
      return
    }
    $key = $line.Substring(0, $separator).Trim()
    $value = $line.Substring($separator + 1).Trim()
    if (
      $value.Length -ge 2 -and
      (($value.StartsWith('"') -and $value.EndsWith('"')) -or
        ($value.StartsWith("'") -and $value.EndsWith("'")))
    ) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    if ($key) {
      $values[$key] = $value
    }
  }
  return $values
}

function New-EvaluationToken {
  $bytes = [byte[]]::new(32)
  [Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
  return [Convert]::ToHexString($bytes).ToLowerInvariant()
}

function Protect-Text {
  param(
    [AllowNull()][string]$Text,
    [Parameter(Mandatory = $true)][string[]]$Secrets
  )

  if ($null -eq $Text) {
    return ""
  }
  $safe = $Text
  foreach ($secret in $Secrets) {
    if (-not [string]::IsNullOrWhiteSpace($secret)) {
      $safe = $safe.Replace($secret, "<redacted>")
    }
  }
  return $safe -replace '(?i)Bearer\s+[A-Za-z0-9._~+/=-]+', 'Bearer <redacted>'
}

function Protect-File {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string[]]$Secrets
  )

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return
  }
  $text = [IO.File]::ReadAllText($Path)
  $safe = Protect-Text $text $Secrets
  if ($safe -ne $text) {
    [IO.File]::WriteAllText($Path, $safe, [Text.UTF8Encoding]::new($false))
  }
}

function Test-FilesForSecret {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string[]]$Secrets
  )

  foreach ($file in Get-ChildItem -LiteralPath $Root -File -Recurse) {
    $content = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($file.FullName))
    foreach ($secret in $Secrets) {
      if (
        -not [string]::IsNullOrWhiteSpace($secret) -and
        $content.IndexOf($secret, [StringComparison]::Ordinal) -ge 0
      ) {
        return $false
      }
    }
  }
  return $true
}

function Protect-HarnessArtifacts {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string[]]$Secrets
  )

  if (-not (Test-Path -LiteralPath $Root -PathType Container)) {
    return
  }
  Get-ChildItem -LiteralPath $Root -File -Recurse -ErrorAction SilentlyContinue |
    Where-Object { $_.Extension -in @(".json", ".jsonl", ".log", ".md") } |
    ForEach-Object { Protect-File $_.FullName $Secrets }
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
    $process.Refresh()
    if ($process.HasExited) {
      throw "OpenTopia server exited before becoming healthy"
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
  Stop-EvalServer $process
  throw "OpenTopia server did not become healthy within 30 seconds"
}

function Stop-EvalServer {
  param([AllowNull()][System.Diagnostics.Process]$Process)

  if ($Process) {
    $Process.Refresh()
  }
  if ($Process -and -not $Process.HasExited) {
    Stop-Process -Id $Process.Id -Force
    $Process.WaitForExit(5000) | Out-Null
  }
}

function Start-BrowserFixture {
  param(
    [Parameter(Mandatory = $true)][string]$FixturePath,
    [Parameter(Mandatory = $true)][string]$StatePath,
    [Parameter(Mandatory = $true)][string]$RunRoot
  )

  $nodePath = (Get-Command node -ErrorAction Stop).Source
  $stdoutPath = Join-Path $RunRoot "browser-fixture.stdout.log"
  $stderrPath = Join-Path $RunRoot "browser-fixture.stderr.log"
  $process = Start-Process `
    -FilePath $nodePath `
    -ArgumentList @($FixturePath, "--host", "127.0.0.1", "--port", $BrowserFixturePort, "--state", $StatePath) `
    -RedirectStandardOutput $stdoutPath `
    -RedirectStandardError $stderrPath `
    -PassThru `
    -WindowStyle Hidden

  $deadline = (Get-Date).AddSeconds(15)
  $fixtureUrl = "http://127.0.0.1:$BrowserFixturePort"
  while ((Get-Date) -lt $deadline) {
    $process.Refresh()
    if ($process.HasExited) {
      throw "Browser fixture exited before becoming healthy"
    }
    Start-Sleep -Milliseconds 150
    try {
      $health = Invoke-RestMethod -Uri "$fixtureUrl/health" -TimeoutSec 2
      if ($health.ok -and $health.service -eq "opentopia-browser-fixture") {
        return $process
      }
    } catch {
    }
  }
  Stop-EvalServer $process
  throw "Browser fixture did not become healthy within 15 seconds"
}

function Read-RestartControl {
  param([Parameter(Mandatory = $true)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    return $null
  }
  try {
    $raw = [IO.File]::ReadAllText($Path)
    if ([string]::IsNullOrWhiteSpace($raw)) {
      return $null
    }
    $value = $raw | ConvertFrom-Json
    $action = [string]$value.action
    $status = [string]$value.status
    if ($action -eq "restart" -and $status -notin @("completed", "failed")) {
      return $value
    }
  } catch {
    # The adapter may be between opening and replacing the control file.
  }
  return $null
}

function Write-RestartControlResult {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][object]$Request,
    [Parameter(Mandatory = $true)][string]$Status,
    [AllowNull()][string]$Error = $null
  )

  $result = [ordered]@{
    action = "restart"
    requestId = [string]$Request.requestId
    status = $Status
    completedAt = (Get-Date).ToUniversalTime().ToString("o")
  }
  if ($Error) {
    $result.error = $Error
  }
  $temporary = "$Path.tmp"
  [IO.File]::WriteAllText(
    $temporary,
    (($result | ConvertTo-Json -Compress) + "`n"),
    [Text.UTF8Encoding]::new($false)
  )
  Move-Item -LiteralPath $temporary -Destination $Path -Force
}

function Update-RestartControl {
  param(
    [Parameter(Mandatory = $true)][string]$PrimaryPath,
    [Parameter(Mandatory = $true)][string]$ServerPath,
    [Parameter(Mandatory = $true)][string]$DatabasePath,
    [Parameter(Mandatory = $true)][string]$RunRoot
  )

  $request = Read-RestartControl $PrimaryPath
  if (-not $request) {
    return
  }
  $script:RestartCount += 1
  try {
    Stop-EvalServer $script:ServerProcess
    $script:ServerProcess = Start-EvalServer `
      "restart-$script:RestartCount" `
      $ServerPath `
      $DatabasePath `
      $RunRoot
    Write-RestartControlResult $PrimaryPath $request "completed"
  } catch {
    $errorText = Protect-Text $_.Exception.Message $script:Secrets
    Write-RestartControlResult $PrimaryPath $request "failed" $errorText
    throw "OpenTopia server restart failed: $errorText"
  }
}

function New-RuntimeTarget {
  param(
    [Parameter(Mandatory = $true)][string]$SourcePath,
    [Parameter(Mandatory = $true)][string]$DestinationPath,
    [Parameter(Mandatory = $true)][string]$AdapterPath
  )

  $target = Get-Content -Raw -Encoding UTF8 -LiteralPath $SourcePath | ConvertFrom-Json
  if (-not $target.env) {
    $target | Add-Member -NotePropertyName env -NotePropertyValue ([PSCustomObject]@{})
  }
  $baseUrlProperty = $target.env.PSObject.Properties["OPENTOPIA_EVAL_BASE_URL"]
  if ($baseUrlProperty) {
    $baseUrlProperty.Value = "http://127.0.0.1:$Port"
  } else {
    $target.env | Add-Member -NotePropertyName "OPENTOPIA_EVAL_BASE_URL" -NotePropertyValue "http://127.0.0.1:$Port"
  }
  $target.args = @($target.args | ForEach-Object {
    $_.Replace("{targetDir}/../../adapters/opentopia-http.mjs", $AdapterPath)
  })
  $targetJson = $target | ConvertTo-Json -Depth 20
  [IO.File]::WriteAllText($DestinationPath, "$targetJson`n", [Text.UTF8Encoding]::new($false))
}

$repoRoot = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\dev-env.ps1"

$envFilePath = (Resolve-Path -LiteralPath $EnvFile).Path
$values = ConvertFrom-DotEnvFile $envFilePath
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "The selected provider profile is incomplete"
}
if ($ExpectedModel -and $model -ne $ExpectedModel) {
  throw "Selected model does not match the expected model"
}

$suitePath = if ([IO.Path]::IsPathRooted($SuitePath)) {
  $SuitePath
} else {
  Join-Path $repoRoot $SuitePath
}
$suitePath = (Resolve-Path -LiteralPath $suitePath).Path
$targetPath = if ($TargetPath) {
  if ([IO.Path]::IsPathRooted($TargetPath)) {
    $TargetPath
  } else {
    Join-Path $repoRoot $TargetPath
  }
} else {
  Join-Path (Split-Path -Parent $suitePath) "target.json"
}
$targetPath = (Resolve-Path -LiteralPath $targetPath).Path
$adapterPath = Join-Path $repoRoot "evaluation\adapters\opentopia-http.mjs"
foreach ($path in @($suitePath, $targetPath, $adapterPath)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
    throw "Required evaluation file was not found: $path"
  }
}
$suite = Get-Content -Raw -Encoding UTF8 -LiteralPath $suitePath | ConvertFrom-Json
$suiteId = [string]$suite.id
if ([string]::IsNullOrWhiteSpace($suiteId)) {
  throw "Evaluation suite must declare a non-empty id"
}
$safeSuiteId = ($suiteId.ToLowerInvariant() -replace '[^a-z0-9]+', '-').Trim('-')

$savedEnvironment = @{}
foreach ($name in @(
  "OPENTOPIA_API_KEY",
  "OPENTOPIA_OPENAI_BASE_URL",
  "OPENTOPIA_MODEL",
  "OPENTOPIA_API_TOKEN",
  "OPENTOPIA_DB",
  "OPENTOPIA_SANDBOX_MODE",
  "OPENTOPIA_SANDBOX_ENFORCEMENT",
  "OPENTOPIA_SANDBOX_NETWORK",
  "OPENTOPIA_EVAL_RESTART_CONTROL",
  "OPENTOPIA_BROWSER_DATA_ROOT",
  "OPENTOPIA_EVAL_BROWSER_FIXTURE_URL",
  "OPENTOPIA_EVAL_BROWSER_FIXTURE_STATE",
  "OPENTOPIA_EVAL_BROWSER_DATA_ROOT"
)) {
  $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$evaluationToken = New-EvaluationToken
$script:Secrets = @($apiKey, $evaluationToken)
[Environment]::SetEnvironmentVariable("OPENTOPIA_API_KEY", $apiKey, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_OPENAI_BASE_URL", $baseUrl, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_MODEL", $model, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_API_TOKEN", $evaluationToken, "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_MODE", "workspace-write", "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_ENFORCEMENT", "best-effort", "Process")
[Environment]::SetEnvironmentVariable("OPENTOPIA_SANDBOX_NETWORK", "deny", "Process")
$script:ApiHeaders = @{ Authorization = "Bearer $evaluationToken" }

$runId = "$safeSuiteId-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$runRoot = Join-Path $repoRoot ".opentopia\evaluations\$runId"
$harnessOutput = if ($OutputDirectory) {
  if ([IO.Path]::IsPathRooted($OutputDirectory)) {
    $OutputDirectory
  } else {
    Join-Path $repoRoot $OutputDirectory
  }
} else {
  Join-Path $runRoot "harness-runs"
}
$databasePath = Join-Path $runRoot "evaluation.db"
$runtimeTargetPath = Join-Path $runRoot "target.json"
$restartControlPath = Join-Path $runRoot "restart-control.json"
$harnessStdoutPath = Join-Path $runRoot "harness.stdout.log"
$harnessStderrPath = Join-Path $runRoot "harness.stderr.log"
$resultPath = Join-Path $runRoot "runner-result.json"
$script:ServerProcess = $null
$script:BrowserFixtureProcess = $null
$script:RestartCount = 0
$harnessProcess = $null
$providerHealth = $null
$runError = $null
$harnessExitCode = $null
$startedAt = Get-Date

New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
New-Item -ItemType Directory -Path $harnessOutput -Force | Out-Null
[Environment]::SetEnvironmentVariable(
  "OPENTOPIA_EVAL_RESTART_CONTROL",
  $restartControlPath,
  "Process"
)
if ($BrowserFixture) {
  $browserDataRoot = Join-Path $runRoot "browser-data"
  $browserFixtureStatePath = Join-Path $runRoot "browser-fixture-state.json"
  New-Item -ItemType Directory -Path $browserDataRoot -Force | Out-Null
  [Environment]::SetEnvironmentVariable("OPENTOPIA_BROWSER_DATA_ROOT", $browserDataRoot, "Process")
  [Environment]::SetEnvironmentVariable("OPENTOPIA_EVAL_BROWSER_DATA_ROOT", $browserDataRoot, "Process")
  [Environment]::SetEnvironmentVariable(
    "OPENTOPIA_EVAL_BROWSER_FIXTURE_URL",
    "http://127.0.0.1:$BrowserFixturePort",
    "Process"
  )
  [Environment]::SetEnvironmentVariable("OPENTOPIA_EVAL_BROWSER_FIXTURE_STATE", $browserFixtureStatePath, "Process")
}

try {
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
  if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
    $serverPath = Join-Path $targetDir "debug\opentopia-server"
  }
  if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
    throw "opentopia-server debug binary was not found"
  }

  if ($BrowserFixture) {
    $fixturePath = Join-Path $repoRoot "evaluation\fixtures\browser-fixture-server.mjs"
    if (-not (Test-Path -LiteralPath $fixturePath -PathType Leaf)) {
      throw "Browser fixture server was not found: $fixturePath"
    }
    $script:BrowserFixtureProcess = Start-BrowserFixture $fixturePath $browserFixtureStatePath $runRoot
  }

  New-RuntimeTarget $targetPath $runtimeTargetPath $adapterPath
  $script:ServerProcess = Start-EvalServer "initial" $serverPath $databasePath $runRoot
  $providerHealth = Invoke-EvalApi "Post" "/api/provider/test" @{}
  if (-not $providerHealth.reachable -or -not $providerHealth.modelAvailable) {
    throw "OpenTopia provider health check failed"
  }
  $settings = Invoke-EvalApi "Get" "/api/settings"
  $activeProvider = @($settings.providers | Where-Object {
    $_.id -eq $settings.activeProviderId
  })[0]
  if (-not $activeProvider) {
    $activeProvider = @($settings.providers | Where-Object { $_.model -eq $model })[0]
  }
  if (
    -not $activeProvider -or
    $activeProvider.model -ne $model -or
    $activeProvider.baseUrl.TrimEnd("/") -ne $baseUrl
  ) {
    throw "OpenTopia active provider settings do not match the selected profile"
  }

  $nodePath = (Get-Command node -ErrorAction Stop).Source
  $harnessProcess = Start-Process `
    -FilePath $nodePath `
    -ArgumentList @(
      "evaluation/src/cli.mjs",
      "run",
      "--suite", $suitePath,
      "--target", $runtimeTargetPath,
      "--output", $harnessOutput,
      "--repetitions", $Repetitions
    ) `
    -WorkingDirectory $repoRoot `
    -RedirectStandardOutput $harnessStdoutPath `
    -RedirectStandardError $harnessStderrPath `
    -PassThru `
    -WindowStyle Hidden

  while ($true) {
    $harnessProcess.Refresh()
    if ($harnessProcess.HasExited) {
      break
    }
    Update-RestartControl `
      $restartControlPath `
      $serverPath `
      $databasePath `
      $runRoot
    Start-Sleep -Milliseconds 100
  }
  $harnessExitCode = $harnessProcess.ExitCode
} catch {
  $runError = Protect-Text $_.Exception.Message $script:Secrets
  if ($harnessProcess) {
    $harnessProcess.Refresh()
    if (-not $harnessProcess.HasExited) {
      Stop-Process -Id $harnessProcess.Id -Force
      $harnessProcess.WaitForExit(5000) | Out-Null
    }
    $harnessExitCode = $harnessProcess.ExitCode
  }
} finally {
  Stop-EvalServer $script:ServerProcess
  Stop-EvalServer $script:BrowserFixtureProcess
  Protect-File $harnessStdoutPath $script:Secrets
  Protect-File $harnessStderrPath $script:Secrets
  Get-ChildItem -LiteralPath $runRoot -File -Filter "server-*.log" -ErrorAction SilentlyContinue |
    ForEach-Object { Protect-File $_.FullName $script:Secrets }
  Get-ChildItem -LiteralPath $runRoot -File -Filter "browser-fixture*.log" -ErrorAction SilentlyContinue |
    ForEach-Object { Protect-File $_.FullName $script:Secrets }
  Protect-HarnessArtifacts $harnessOutput $script:Secrets
  foreach ($name in $savedEnvironment.Keys) {
    [Environment]::SetEnvironmentVariable($name, $savedEnvironment[$name], "Process")
  }
}

$secretAuditPassed =
  (Test-FilesForSecret $runRoot $script:Secrets) -and
  (Test-FilesForSecret $harnessOutput $script:Secrets)
$status = if ($runError) {
  "infra_error"
} elseif ($harnessExitCode -eq 0 -and $secretAuditPassed) {
  "passed"
} elseif (-not $secretAuditPassed) {
  "safety_violation"
} else {
  "failed"
}
$result = [ordered]@{
  schemaVersion = 1
  runId = $runId
  startedAt = $startedAt.ToUniversalTime().ToString("o")
  completedAt = (Get-Date).ToUniversalTime().ToString("o")
  status = $status
  provider = [ordered]@{
    profile = $Profile
    expectedModel = if ($ExpectedModel) { $ExpectedModel } else { $null }
    model = $model
    baseUrl = $baseUrl
    credentials = "redacted:set"
    reachable = if ($providerHealth) { $providerHealth.reachable } else { $false }
    modelAvailable = if ($providerHealth) { $providerHealth.modelAvailable } else { $false }
  }
  configuration = [ordered]@{
    suite = $suitePath.Substring($repoRoot.Length).TrimStart('\', '/')
    target = $targetPath.Substring($repoRoot.Length).TrimStart('\', '/')
    repetitions = $Repetitions
    port = $Port
    database = ".opentopia/evaluations/$runId/evaluation.db"
    browserFixture = if ($BrowserFixture) {
      [ordered]@{
        url = "http://127.0.0.1:$BrowserFixturePort"
        state = ".opentopia/evaluations/$runId/browser-fixture-state.json"
        dataRoot = ".opentopia/evaluations/$runId/browser-data"
      }
    } else {
      $null
    }
    sandbox = [ordered]@{
      mode = "workspace-write"
      enforcement = "best-effort"
      network = "deny"
    }
  }
  lifecycle = [ordered]@{
    restartControl = ".opentopia/evaluations/$runId/restart-control.json"
    serverRestarts = $script:RestartCount
  }
  harness = [ordered]@{
    exitCode = $harnessExitCode
    outputDirectory = $harnessOutput
    stdout = ".opentopia/evaluations/$runId/harness.stdout.log"
    stderr = ".opentopia/evaluations/$runId/harness.stderr.log"
  }
  secretAuditPassed = $secretAuditPassed
  error = $runError
}
$resultJson = $result | ConvertTo-Json -Depth 20
if ($resultJson.Contains($apiKey) -or $resultJson.Contains($evaluationToken)) {
  throw "Secret audit rejected final evaluation report"
}
[IO.File]::WriteAllText($resultPath, "$resultJson`n", [Text.UTF8Encoding]::new($false))
if ($SummaryPath) {
  $summaryParent = Split-Path -Parent $SummaryPath
  if ($summaryParent) {
    New-Item -ItemType Directory -Path $summaryParent -Force | Out-Null
  }
  [IO.File]::WriteAllText(
    $SummaryPath,
    "$resultJson`n",
    [Text.UTF8Encoding]::new($false)
  )
}

$harnessOutputText = if (Test-Path -LiteralPath $harnessStdoutPath) {
  [IO.File]::ReadAllText($harnessStdoutPath)
} else {
  ""
}
if ($harnessOutputText) {
  Write-Output $harnessOutputText.TrimEnd()
}
$resultJson
if ($status -ne "passed") {
  exit 1
}
