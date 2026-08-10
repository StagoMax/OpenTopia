param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "",
  [string]$ModelOverride = "",
  [string]$BaseUrlOverride = "",
  [string]$ProviderId = "",
  [string]$ReasoningEffort = "",
  [ValidateRange(1024, 65535)][int]$Port = 8812,
  [ValidateRange(1, 100)][int]$Repetitions = 1,
  [string]$SuitePath = "evaluation\examples\opentopia-tool-suite\suite.json",
  [string]$TargetPath = "",
  [string]$OutputDirectory = "",
  [string]$SummaryPath = "",
  [string]$ExperimentPath = "",
  [string]$ExperimentId = "",
  [string]$PairingKey = "",
  [ValidateSet("", "baseline", "candidate", "control", "treatment")]
  [string]$Variant = "",
  [string]$TreatmentLabel = "",
  [string]$TaskIds = "",
  [switch]$VisibleInDesktop,
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
  $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
  try {
    $generator.GetBytes($bytes)
  } finally {
    $generator.Dispose()
  }
  return ([BitConverter]::ToString($bytes) -replace '-', '').ToLowerInvariant()
}

function Get-TextSha256 {
  param([AllowEmptyString()][string]$Text)

  $bytes = [Text.Encoding]::UTF8.GetBytes($Text)
  $algorithm = [Security.Cryptography.SHA256]::Create()
  try {
    return ([BitConverter]::ToString($algorithm.ComputeHash($bytes)) -replace '-', '').ToLowerInvariant()
  } finally {
    $algorithm.Dispose()
  }
}

function Get-ContentTreeSha256 {
  param([Parameter(Mandatory = $true)][string[]]$Paths)

  $files = @($Paths | ForEach-Object {
    if (Test-Path -LiteralPath $_ -PathType Container) {
      Get-ChildItem -LiteralPath $_ -Recurse -File
    } elseif (Test-Path -LiteralPath $_ -PathType Leaf) {
      Get-Item -LiteralPath $_
    }
  } | Sort-Object FullName)
  if (-not $files) {
    throw "No prompt source files were found"
  }
  $content = [Text.StringBuilder]::new()
  foreach ($file in $files) {
    [void]$content.Append($file.FullName)
    [void]$content.Append([char]0)
    [void]$content.Append([IO.File]::ReadAllText($file.FullName))
    [void]$content.Append([char]0)
  }
  return Get-TextSha256 $content.ToString()
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
  $text = $null
  for ($attempt = 1; $attempt -le 10; $attempt += 1) {
    try {
      $text = [IO.File]::ReadAllText($Path)
      break
    } catch [IO.IOException] {
      if ($attempt -eq 10) {
        throw
      }
      Start-Sleep -Milliseconds 100
    }
  }
  $safe = Protect-Text $text $Secrets
  if ($safe -ne $text) {
    for ($attempt = 1; $attempt -le 10; $attempt += 1) {
      try {
        [IO.File]::WriteAllText($Path, $safe, [Text.UTF8Encoding]::new($false))
        break
      } catch [IO.IOException] {
        if ($attempt -eq 10) {
          throw
        }
        Start-Sleep -Milliseconds 100
      }
    }
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
    [Parameter(Mandatory = $true)][string]$AdapterPath,
    [AllowEmptyString()][string]$ProviderId = "",
    [AllowEmptyString()][string]$ModelId = "",
    [AllowEmptyString()][string]$ReasoningEffort = ""
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
  if ($ProviderId -and $ModelId) {
    $target.env | Add-Member -NotePropertyName "OPENTOPIA_EVAL_PROVIDER_ID" -NotePropertyValue $ProviderId -Force
    $target.env | Add-Member -NotePropertyName "OPENTOPIA_EVAL_MODEL_ID" -NotePropertyValue $ModelId -Force
    $target.env | Add-Member -NotePropertyName "OPENTOPIA_EVAL_TITLE_PREFIX" -NotePropertyValue "Kimi-k3 Architecture Eval" -Force
    if ($ReasoningEffort) {
      $target.env | Add-Member -NotePropertyName "OPENTOPIA_EVAL_REASONING_EFFORT" -NotePropertyValue $ReasoningEffort -Force
    }
  }
  $target.args = @($target.args | ForEach-Object {
    $_.Replace("{targetDir}/../../adapters/opentopia-http.mjs", $AdapterPath)
  })
  $targetJson = $target | ConvertTo-Json -Depth 20
  [IO.File]::WriteAllText($DestinationPath, "$targetJson`n", [Text.UTF8Encoding]::new($false))
}

function Set-EvaluationProviderSettings {
  param(
    [Parameter(Mandatory = $true)][string]$Model,
    [AllowEmptyString()][string]$ReasoningEffort = ""
  )

  $settings = Invoke-EvalApi "Get" "/api/settings"
  $activeProviderId = [string]$settings.activeProviderId
  $providers = @($settings.providers | ForEach-Object {
    if ([string]$_.id -eq $activeProviderId) {
      $_.model = $Model
      if ($ReasoningEffort) {
        $_.reasoningEffort = $ReasoningEffort
      }
    }
    $_
  })
  if (-not $providers -or -not ($providers | Where-Object { [string]$_.id -eq $activeProviderId })) {
    throw "OpenTopia active provider was not found"
  }

  Invoke-EvalApi "Patch" "/api/settings" @{
    providers = $providers
    activeProviderId = $activeProviderId
  } | Out-Null
  return Invoke-EvalApi "Get" "/api/settings"
}

$repoRoot = Split-Path -Parent $PSScriptRoot
. "$PSScriptRoot\dev-env.ps1"

$envFilePath = (Resolve-Path -LiteralPath $EnvFile).Path
$values = ConvertFrom-DotEnvFile $envFilePath
$apiKey = @(
  [string]$values["${Profile}_API_KEY"]
  [string]$values["${Profile}_KEY"]
  [Environment]::GetEnvironmentVariable("${Profile}_API_KEY", "Process")
  [Environment]::GetEnvironmentVariable("${Profile}_KEY", "Process")
) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1
$baseUrl = @(
  [string]$values["${Profile}_BASE_URL"]
  [Environment]::GetEnvironmentVariable("${Profile}_BASE_URL", "Process")
) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1
$configuredModel = @(
  [string]$values["${Profile}_MODEL"]
  [string]$values["${Profile}MODEL"]
  [Environment]::GetEnvironmentVariable("${Profile}_MODEL", "Process")
  [Environment]::GetEnvironmentVariable("${Profile}MODEL", "Process")
) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1
$apiKey = [string]$apiKey
$baseUrl = if ($BaseUrlOverride) {
  $BaseUrlOverride.TrimEnd("/")
} else {
  ([string]$baseUrl).TrimEnd("/")
}
$configuredModel = [string]$configuredModel
$model = if ($ModelOverride) { $ModelOverride.Trim() } else { $configuredModel }
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "The selected provider profile is incomplete"
}
if ($ExpectedModel -and $model -ne $ExpectedModel) {
  throw "Selected model does not match the expected model"
}
$supportedReasoningEfforts = @("none", "minimal", "low", "medium", "high", "xhigh", "max")
if ($ReasoningEffort -and $supportedReasoningEfforts -notcontains $ReasoningEffort) {
  throw "ReasoningEffort must be one of: $($supportedReasoningEfforts -join ', ')"
}
if ($ExperimentPath -and ($ExperimentId -or $PairingKey -or $Variant)) {
  throw "Use either ExperimentPath or the generated ExperimentId/PairingKey/Variant parameters"
}
if (($ExperimentId -or $PairingKey -or $Variant) -and -not ($ExperimentId -and $PairingKey -and $Variant)) {
  throw "ExperimentId, PairingKey, and Variant must be supplied together"
}
if ($VisibleInDesktop -and -not $ProviderId) {
  throw "ProviderId is required when VisibleInDesktop is enabled"
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
  "OPENTOPIA_EVAL_BROWSER_DATA_ROOT",
  "OPENTOPIA_WINDOWS_SANDBOX_BIN"
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
$sandboxBinary = Join-Path $repoRoot ".opentopia\verify-target\debug\opentopia-sandbox.exe"
[Environment]::SetEnvironmentVariable("OPENTOPIA_WINDOWS_SANDBOX_BIN", $sandboxBinary, "Process")
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
$databasePath = if ($VisibleInDesktop) {
  Join-Path $repoRoot ".opentopia\opentopia.db"
} else {
  Join-Path $runRoot "evaluation.db"
}
$runtimeTargetPath = Join-Path $runRoot "target.json"
$restartControlPath = Join-Path $runRoot "restart-control.json"
$harnessStdoutPath = Join-Path $runRoot "harness.stdout.log"
$harnessStderrPath = Join-Path $runRoot "harness.stderr.log"
$resultPath = Join-Path $runRoot "runner-result.json"
$runtimeExperimentPath = $null
$runtimeExperiment = $null
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
      cargo build -p opentopia-server -p opentopia-windows-sandbox
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

  New-RuntimeTarget $targetPath $runtimeTargetPath $adapterPath $ProviderId $model $ReasoningEffort
  $script:ServerProcess = Start-EvalServer "initial" $serverPath $databasePath $runRoot
  if ($VisibleInDesktop) {
    $settings = Invoke-EvalApi "Get" "/api/settings"
    $activeProvider = @($settings.providers | Where-Object { $_.id -eq $ProviderId })[0]
    if (-not $activeProvider) {
      throw "Configured desktop provider was not found: $ProviderId"
    }
    if (@($activeProvider.syncedModels) -notcontains $model -and $activeProvider.model -ne $model) {
      throw "Model $model is not available on desktop provider $ProviderId"
    }
    $providerHealth = Invoke-EvalApi "Post" "/api/provider/test" @{ providerId = $ProviderId } 90
  } else {
    $settings = Set-EvaluationProviderSettings $model $ReasoningEffort
    $providerHealth = Invoke-EvalApi "Post" "/api/provider/test" @{}
  }
  if (-not $providerHealth.reachable -or -not $providerHealth.modelAvailable) {
    throw "OpenTopia provider health check failed"
  }
  $settings = Invoke-EvalApi "Get" "/api/settings"
  $activeProvider = @($settings.providers | Where-Object {
    $_.id -eq $(if ($VisibleInDesktop) { $ProviderId } else { $settings.activeProviderId })
  })[0]
  if (-not $activeProvider) {
    $activeProvider = @($settings.providers | Where-Object { $_.model -eq $model })[0]
  }
  if (
    -not $activeProvider -or
    ((-not $VisibleInDesktop) -and $activeProvider.model -ne $model) -or
    $activeProvider.baseUrl.TrimEnd("/") -ne $baseUrl -or
    ($ReasoningEffort -and $activeProvider.reasoningEffort -ne $ReasoningEffort)
  ) {
    throw "OpenTopia active provider settings do not match the selected evaluation configuration"
  }

  if ($ExperimentPath) {
    $candidateExperimentPath = if ([IO.Path]::IsPathRooted($ExperimentPath)) {
      $ExperimentPath
    } else {
      Join-Path $repoRoot $ExperimentPath
    }
    $runtimeExperimentPath = (Resolve-Path -LiteralPath $candidateExperimentPath).Path
    $runtimeExperiment = Get-Content -LiteralPath $runtimeExperimentPath -Raw | ConvertFrom-Json
  } elseif ($ExperimentId) {
    $agentSourcePath = Join-Path $repoRoot "crates\opentopia-core\src\agent.rs"
    $basePromptModulePath = Join-Path $repoRoot "crates\opentopia-core\src\base_prompt.rs"
    $basePromptDirectory = Join-Path $repoRoot "crates\opentopia-core\src\prompts\base"
    $serverSourcePath = Join-Path $repoRoot "crates\opentopia-server\src\main.rs"
    $basePromptVersion = [regex]::Match(
      [IO.File]::ReadAllText($agentSourcePath),
      'BASE_AGENT_PROMPT_VERSION:\s*&str\s*=\s*"([^"]+)"'
    ).Groups[1].Value
    $gitRevision = (& git -C $repoRoot rev-parse HEAD 2>$null).Trim()
    $gitDirty = [bool](& git -C $repoRoot status --porcelain 2>$null)
    $agentRuntimeJson = if ($settings.agentRuntime) {
      $settings.agentRuntime | ConvertTo-Json -Compress -Depth 20
    } else {
      "{}"
    }
    $experiment = [ordered]@{
      schemaVersion = 1
      experimentId = $ExperimentId
      pairingKey = $PairingKey
      variant = $Variant
      controlled = [ordered]@{
        suite = $suiteId
        providerProfile = $Profile
        model = $model
        reasoningEffort = if ($ReasoningEffort) { $ReasoningEffort } else { [string]$activeProvider.reasoningEffort }
        repetitions = $Repetitions
        sandbox = [ordered]@{
          mode = "workspace-write"
          network = "deny"
        }
      }
      treatment = [ordered]@{
        label = $TreatmentLabel
        gitRevision = $gitRevision
        gitDirty = $gitDirty
        basePromptVersion = $basePromptVersion
        basePromptSha256 = Get-ContentTreeSha256 @($basePromptModulePath, $basePromptDirectory)
        agentRuntimeSha256 = Get-TextSha256 $agentRuntimeJson
        coreAgentSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $agentSourcePath).Hash.ToLowerInvariant()
        serverSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $serverSourcePath).Hash.ToLowerInvariant()
        adapterSha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $adapterPath).Hash.ToLowerInvariant()
      }
      notes = "Generated by evaluate-opentopia-tool-suite.ps1 from the effective evaluation configuration."
    }
    $runtimeExperiment = $experiment
    $runtimeExperimentPath = Join-Path $runRoot "experiment.json"
    [IO.File]::WriteAllText(
      $runtimeExperimentPath,
      "$(($experiment | ConvertTo-Json -Depth 20))`n",
      [Text.UTF8Encoding]::new($false)
    )
  }

  $nodePath = (Get-Command node -ErrorAction Stop).Source
  $harnessArguments = @(
    "evaluation/src/cli.mjs",
    "run",
    "--suite", $suitePath,
    "--target", $runtimeTargetPath,
    "--output", $harnessOutput,
    "--repetitions", $Repetitions
  )
  if ($runtimeExperimentPath) {
    $harnessArguments += @("--experiment", $runtimeExperimentPath)
  }
  if ($TaskIds) {
    $harnessArguments += @("--tasks", $TaskIds)
  }
  $harnessProcess = Start-Process `
    -FilePath $nodePath `
    -ArgumentList $harnessArguments `
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
$runtimeExperimentDisplayPath = if (
  $runtimeExperimentPath -and
  $runtimeExperimentPath.StartsWith($repoRoot, [StringComparison]::OrdinalIgnoreCase)
) {
  $runtimeExperimentPath.Substring($repoRoot.Length).TrimStart('\', '/')
} else {
  $runtimeExperimentPath
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
    configuredModel = $configuredModel
    reasoningEffort = if ($ReasoningEffort) { $ReasoningEffort } else { $null }
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
    database = if ($VisibleInDesktop) { ".opentopia/opentopia.db" } else { ".opentopia/evaluations/$runId/evaluation.db" }
    desktopVisible = [bool]$VisibleInDesktop
    selectedTaskIds = if ($TaskIds) {
      @($TaskIds.Split(',') | ForEach-Object { $_.Trim() } | Where-Object { $_ })
    } else {
      $null
    }
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
    experiment = if ($runtimeExperimentPath) {
      [ordered]@{
        path = $runtimeExperimentDisplayPath
        experimentId = [string]$runtimeExperiment.experimentId
        pairingKey = [string]$runtimeExperiment.pairingKey
        variant = [string]$runtimeExperiment.variant
      }
    } else {
      $null
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
