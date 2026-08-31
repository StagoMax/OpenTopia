param(
  [string]$EnvFile = "J:\Project\OpenTopia-eval-secrets\nowcoding-gpt-5.6-terra.env",
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$HarnessRoot = "J:\Project\HarnessBench",
  [string]$PlanRoot = "J:\Project\OpenTopia-evaluation-results\external-agent-benchmark-plan-20260824",
  [string]$AfterArtifactPath = "",
  [string]$OutputRoot = "",
  [string]$WorkRoot = "",
  [string]$SelectionFile = "",
  [string]$StatusFile = "pilot-status.json",
  [string]$SummaryFile = "pilot-summary.json",
  [string]$ReportFile = "第一阶段试点报告.md",
  [string]$TaskIds = "",
  [ValidateRange(1, 8)][int]$BeforeConcurrency = 2,
  [ValidateRange(1, 8)][int]$AfterConcurrency = 2,
  [ValidateRange(0, 600)][int]$InvalidRetryDelaySec = 60,
  [ValidateRange(1024, 65535)][int]$BeforePort = 18812,
  [ValidateRange(1024, 65535)][int]$AfterPort = 18813,
  [switch]$SkipImageBuild
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#")) { return }
    if ($line.StartsWith("export ")) { $line = $line.Substring(7).Trim() }
    $separator = $line.IndexOf("=")
    if ($separator -le 0) { return }
    $key = $line.Substring(0, $separator).Trim()
    $value = $line.Substring($separator + 1).Trim()
    if (
      $value.Length -ge 2 -and
      (($value.StartsWith('"') -and $value.EndsWith('"')) -or
        ($value.StartsWith("'") -and $value.EndsWith("'")))
    ) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    if ($key) { $values[$key] = $value }
  }
  return $values
}

function New-RandomToken {
  $bytes = [byte[]]::new(32)
  $generator = [Security.Cryptography.RandomNumberGenerator]::Create()
  try { $generator.GetBytes($bytes) } finally { $generator.Dispose() }
  return ([BitConverter]::ToString($bytes) -replace '-', '').ToLowerInvariant()
}

function Wait-ServerHealth {
  param(
    [Parameter(Mandatory = $true)][int]$Port,
    [Parameter(Mandatory = $true)][string]$Token
  )

  $deadline = (Get-Date).AddSeconds(45)
  while ((Get-Date) -lt $deadline) {
    try {
      $health = Invoke-RestMethod `
        -Uri "http://127.0.0.1:$Port/health" `
        -Headers @{ Authorization = "Bearer $Token" } `
        -TimeoutSec 3
      if ($health.ok -and $health.service -eq "opentopia-server") { return }
    } catch {
    }
    Start-Sleep -Milliseconds 500
  }
  throw "OpenTopia shared server on port $Port did not become healthy"
}

function Invoke-ServerApi {
  param(
    [Parameter(Mandatory = $true)][int]$Port,
    [Parameter(Mandatory = $true)][string]$Token,
    [Parameter(Mandatory = $true)][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [AllowNull()][object]$Body = $null
  )

  $parameters = @{
    Method = $Method
    Uri = "http://127.0.0.1:$Port$Path"
    Headers = @{ Authorization = "Bearer $Token" }
    TimeoutSec = 120
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json"
    $parameters.Body = $Body | ConvertTo-Json -Depth 40 -Compress
  }
  return Invoke-RestMethod @parameters
}

function Configure-ServerProvider {
  param(
    [Parameter(Mandatory = $true)][int]$Port,
    [Parameter(Mandatory = $true)][string]$Token,
    [Parameter(Mandatory = $true)][string]$Model
  )

  $settings = Invoke-ServerApi $Port $Token "GET" "/api/settings"
  $providerId = [string]$settings.activeProviderId
  $provider = @($settings.providers | Where-Object { [string]$_.id -eq $providerId }) | Select-Object -First 1
  if (-not $provider) { throw "OpenTopia active provider was not found on port $Port" }
  $provider.model = $Model
  $provider.reasoningEffort = "high"
  $provider.maxOutputTokens = 8192
  $updated = Invoke-ServerApi $Port $Token "PATCH" "/api/settings" @{
    providers = @($settings.providers)
    activeProviderId = $providerId
  }
  $configured = @($updated.providers | Where-Object { [string]$_.id -eq $providerId }) | Select-Object -First 1
  if (
    -not $configured -or
    [string]$configured.model -ne $Model -or
    [string]$configured.reasoningEffort -ne "high"
  ) {
    throw "OpenTopia did not retain the controlled model settings on port $Port"
  }
  $probe = Invoke-ServerApi $Port $Token "POST" "/api/provider/test" @{ providerId = $providerId }
  if (-not $probe.reachable -or -not $probe.modelAvailable) {
    throw "Provider capability probe failed on port $Port"
  }
  return $providerId
}

function Start-SharedServer {
  param(
    [Parameter(Mandatory = $true)][string]$Name,
    [Parameter(Mandatory = $true)][string]$Artifact,
    [Parameter(Mandatory = $true)][string]$Output,
    [Parameter(Mandatory = $true)][string]$Work,
    [Parameter(Mandatory = $true)][int]$Port,
    [Parameter(Mandatory = $true)][string]$Token,
    [Parameter(Mandatory = $true)][string]$Image
  )

  $env:OPENTOPIA_API_TOKEN = $Token
  $arguments = @(
    "run", "--detach",
    "--name", $Name,
    "--label", "opentopia.evaluation=harnessbench-pilot",
    "--publish", "127.0.0.1:${Port}:8812",
    "--mount", "type=bind,source=$Artifact,target=/runtime/opentopia-server,readonly",
    "--mount", "type=bind,source=$Output,target=/bench-output",
    "--mount", "type=bind,source=$Work,target=/bench-work",
    "--env", "OPENTOPIA_API_KEY",
    "--env", "OPENTOPIA_OPENAI_BASE_URL",
    "--env", "OPENTOPIA_MODEL",
    "--env", "OPENTOPIA_API_TOKEN",
    "--env", "OPENTOPIA_SANDBOX_MODE=danger-full-access",
    "--env", "OPENTOPIA_SANDBOX_ENFORCEMENT=disabled",
    "--env", "OPENTOPIA_SANDBOX_NETWORK=inherit",
    $Image,
    "/runtime/opentopia-server",
    "--host", "0.0.0.0",
    "--port", "8812",
    "--db", "/bench-output/runtime/$Name.db",
    "--permission", "full-access"
  )
  $containerId = & docker @arguments
  if ($LASTEXITCODE -ne 0 -or -not $containerId) {
    throw "Failed to start shared OpenTopia container $Name"
  }
  try {
    Wait-ServerHealth $Port $Token
  } catch {
    & docker rm --force $containerId *> $null
    throw
  }
  return [string]$containerId
}

$repoRoot = Split-Path -Parent (Split-Path -Parent $PSScriptRoot)
$envPath = (Resolve-Path -LiteralPath $EnvFile).Path
$harnessPath = (Resolve-Path -LiteralPath $HarnessRoot).Path
$selectionPath = if ($SelectionFile) {
  (Resolve-Path -LiteralPath $SelectionFile).Path
} else {
  Join-Path $repoRoot "evaluation\integrations\harnessbench\pilot_tasks.json"
}
$dockerfile = Join-Path $repoRoot "evaluation\integrations\harnessbench\Dockerfile.server"
$beforeArtifact = Join-Path $PlanRoot "artifacts\opentopia-server-before-6d52-schema-compat-shim-musl"
$afterArtifact = if ($AfterArtifactPath) {
  (Resolve-Path -LiteralPath $AfterArtifactPath).Path
} else {
  Join-Path $PlanRoot "artifacts\opentopia-server-after-e135adbd-worktree-20260831-musl"
}
$afterProvenance = "$afterArtifact.provenance.json"
foreach ($path in @($selectionPath, $dockerfile, $beforeArtifact, $afterArtifact, $afterProvenance)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required file not found: $path" }
}
$beforeArtifactSha256 = (Get-FileHash -LiteralPath $beforeArtifact -Algorithm SHA256).Hash
$afterArtifactSha256 = (Get-FileHash -LiteralPath $afterArtifact -Algorithm SHA256).Hash

$values = ConvertFrom-DotEnvFile $envPath
$apiKey = @(
  [string]$values["${Profile}_API_KEY"]
  [string]$values["${Profile}_KEY"]
) | Where-Object { -not [string]::IsNullOrWhiteSpace($_) } | Select-Object -First 1
$baseUrl = [string]$values["${Profile}_BASE_URL"]
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) { throw "The selected provider profile is incomplete" }
if ($model -ne "gpt-5.6-terra") { throw "The pilot is pinned to gpt-5.6-terra" }

& docker info --format '{{.ServerVersion}}' | Out-Null
if ($LASTEXITCODE -ne 0) { throw "Docker daemon is unavailable" }

$image = "opentopia-harnessbench-runtime:20260831"
if (-not $SkipImageBuild) {
  & docker image inspect $image *> $null
  if ($LASTEXITCODE -ne 0) {
    & docker build --tag $image --file $dockerfile (Split-Path -Parent $dockerfile)
    if ($LASTEXITCODE -ne 0) { throw "Failed to build the Harness-Bench server runtime image" }
  }
}
& docker image inspect $image *> $null
if ($LASTEXITCODE -ne 0) { throw "Harness-Bench server runtime image is unavailable" }

$runStamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$outputPath = if ($OutputRoot) {
  [IO.Path]::GetFullPath($OutputRoot)
} else {
  Join-Path $PlanRoot "harnessbench-pilot-gpt56terra-high-$runStamp"
}
$workPath = if ($WorkRoot) {
  [IO.Path]::GetFullPath($WorkRoot)
} else {
  $driveRoot = [IO.Path]::GetPathRoot($outputPath)
  Join-Path $driveRoot (Join-Path "opentopia-hb-work" (Split-Path -Leaf $outputPath))
}
New-Item -ItemType Directory -Path (Join-Path $outputPath "runtime") -Force | Out-Null
New-Item -ItemType Directory -Path $workPath -Force | Out-Null

$beforeToken = New-RandomToken
$afterToken = New-RandomToken
$beforeName = "opentopia-hb-before-$runStamp".ToLowerInvariant()
$afterName = "opentopia-hb-after-$runStamp".ToLowerInvariant()
$savedEnvironment = @{}
foreach ($name in @(
  "OPENTOPIA_API_KEY",
  "OPENTOPIA_OPENAI_BASE_URL",
  "OPENTOPIA_MODEL",
  "OPENTOPIA_API_TOKEN",
  "OPENTOPIA_HB_BEFORE_TOKEN",
  "OPENTOPIA_HB_AFTER_TOKEN"
)) {
  $savedEnvironment[$name] = [Environment]::GetEnvironmentVariable($name, "Process")
}

$beforeContainer = $null
$afterContainer = $null
try {
  $env:OPENTOPIA_API_KEY = $apiKey
  $env:OPENTOPIA_OPENAI_BASE_URL = $baseUrl.TrimEnd("/")
  $env:OPENTOPIA_MODEL = $model
  $beforeContainer = Start-SharedServer $beforeName $beforeArtifact $outputPath $workPath $BeforePort $beforeToken $image
  $afterContainer = Start-SharedServer $afterName $afterArtifact $outputPath $workPath $AfterPort $afterToken $image
  $beforeProvider = Configure-ServerProvider $BeforePort $beforeToken $model
  $afterProvider = Configure-ServerProvider $AfterPort $afterToken $model

  $env:OPENTOPIA_HB_BEFORE_TOKEN = $beforeToken
  $env:OPENTOPIA_HB_AFTER_TOKEN = $afterToken
  $pythonArguments = @(
    "-m", "evaluation.integrations.harnessbench.run_paired",
    "--harness-root", $harnessPath,
    "--output-root", $outputPath,
    "--work-root", $workPath,
    "--selection", $selectionPath,
    "--status-file", $StatusFile,
    "--summary-file", $SummaryFile,
    "--report-file", $ReportFile,
    "--before-url", "http://127.0.0.1:$BeforePort",
    "--after-url", "http://127.0.0.1:$AfterPort",
    "--before-provider", $beforeProvider,
    "--after-provider", $afterProvider,
    "--before-artifact-sha256", $beforeArtifactSha256,
    "--after-artifact-sha256", $afterArtifactSha256,
    "--model", $model,
    "--reasoning-effort", "high",
    "--before-concurrency", $BeforeConcurrency,
    "--after-concurrency", $AfterConcurrency
    "--invalid-retry-delay-sec", $InvalidRetryDelaySec
  )
  if ($TaskIds) {
    $pythonArguments += @("--task-ids", $TaskIds)
  }
  & python @pythonArguments
  $runnerExitCode = $LASTEXITCODE
  if ($runnerExitCode -notin @(0, 2)) { throw "Harness-Bench paired runner failed with exit code $runnerExitCode" }
  Write-Output "Harness-Bench pilot output: $outputPath"
  if ($runnerExitCode -eq 2) { exit 2 }
} finally {
  foreach ($container in @($beforeContainer, $afterContainer)) {
    if ($container) {
      & docker rm --force $container *> $null
    }
  }
  foreach ($entry in $savedEnvironment.GetEnumerator()) {
    [Environment]::SetEnvironmentVariable($entry.Key, $entry.Value, "Process")
  }
}
