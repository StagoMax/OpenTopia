param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [Parameter(Mandatory = $true)][string]$BeforeRoot,
  [Parameter(Mandatory = $true)][string]$AfterRoot,
  [string]$OutputRoot = "",
  [ValidateRange(1, 10)][int]$Repetitions = 3,
  [switch]$CalibrationOnly,
  [ValidateRange(1024, 65535)][int]$ProxyPort = 9010,
  [ValidateRange(1024, 65535)][int]$ServerPort = 8812,
  [string]$ProviderUpstreamBaseUrl = "https://api.deepseek.com",
  [string]$Model = "deepseek-v4-flash",
  [ValidateSet("none", "minimal", "low", "medium", "high", "xhigh", "max")]
  [string]$ReasoningEffort = "high",
  [double]$InputPricePerMillion = 0.14,
  [double]$CacheHitPricePerMillion = 0.0028,
  [double]$OutputPricePerMillion = 0.28,
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

function Test-TcpPort {
  param([Parameter(Mandatory = $true)][int]$Port)
  $client = [Net.Sockets.TcpClient]::new()
  try {
    $result = $client.BeginConnect("127.0.0.1", $Port, $null, $null)
    if (-not $result.AsyncWaitHandle.WaitOne(500)) { return $false }
    $client.EndConnect($result)
    return $true
  } catch {
    return $false
  } finally {
    $client.Dispose()
  }
}

function Get-TreeHash {
  param([Parameter(Mandatory = $true)][string[]]$Paths)
  $files = @($Paths | ForEach-Object {
    if (Test-Path -LiteralPath $_ -PathType Container) {
      Get-ChildItem -LiteralPath $_ -Recurse -File
    } elseif (Test-Path -LiteralPath $_ -PathType Leaf) {
      Get-Item -LiteralPath $_
    }
  } | Sort-Object FullName)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    foreach ($file in $files) {
      $name = [Text.Encoding]::UTF8.GetBytes($file.FullName + [char]0)
      [void]$sha.TransformBlock($name, 0, $name.Length, $name, 0)
      $bytes = [IO.File]::ReadAllBytes($file.FullName)
      [void]$sha.TransformBlock($bytes, 0, $bytes.Length, $bytes, 0)
    }
    [void]$sha.TransformFinalBlock([byte[]]::new(0), 0, 0)
    return ([BitConverter]::ToString($sha.Hash) -replace '-', '').ToLowerInvariant()
  } finally {
    $sha.Dispose()
  }
}

function Get-SnapshotEvidence {
  param([Parameter(Mandatory = $true)][string]$Root)
  $paths = @(
    (Join-Path $Root "Cargo.toml"),
    (Join-Path $Root "Cargo.lock"),
    (Join-Path $Root "crates\opentopia-core"),
    (Join-Path $Root "crates\opentopia-server"),
    (Join-Path $Root "runtime\office\runtime-lock.json")
  )
  return [ordered]@{
    root = $Root
    gitHead = (& git -C $Root rev-parse HEAD).Trim()
    gitStatus = @(& git -C $Root status --short)
    productTreeSha256 = Get-TreeHash $paths
  }
}

function Build-EvaluationBinaries {
  param([Parameter(Mandatory = $true)][string]$Root)
  $savedTarget = $env:CARGO_TARGET_DIR
  $env:CARGO_TARGET_DIR = Join-Path $Root ".opentopia\verify-target"
  try {
    Push-Location $Root
    try {
      cargo build -p opentopia-server -p opentopia-windows-sandbox
      if ($LASTEXITCODE -ne 0) { throw "evaluation binary build failed: $Root" }
    } finally {
      Pop-Location
    }
  } finally {
    $env:CARGO_TARGET_DIR = $savedTarget
  }
}

$scriptPath = $PSCommandPath
$toolRoot = Split-Path -Parent $scriptPath
$repoRoot = Split-Path -Parent (Split-Path -Parent $toolRoot)
$proxyScript = Join-Path $toolRoot "provider-usage-proxy.mjs"
$usageSummaryScript = Join-Path $toolRoot "summarize-provider-usage.mjs"
$runnerRelativePath = "scripts\evaluate-opentopia-tool-suite.ps1"
$envFilePath = (Resolve-Path -LiteralPath $EnvFile).Path
$beforeRootPath = (Resolve-Path -LiteralPath $BeforeRoot).Path
$afterRootPath = (Resolve-Path -LiteralPath $AfterRoot).Path
foreach ($path in @($proxyScript, $usageSummaryScript, (Join-Path $beforeRootPath $runnerRelativePath), (Join-Path $afterRootPath $runnerRelativePath))) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required evaluation file is missing: $path" }
}

if (-not $OutputRoot) {
  $stamp = (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
  $OutputRoot = Join-Path (Split-Path -Parent $repoRoot) "OpenTopia-evaluation-results\paired-$stamp"
}
$outputRootPath = [IO.Path]::GetFullPath($OutputRoot)
New-Item -ItemType Directory -Path $outputRootPath -Force | Out-Null

$allSuites = @(
  [PSCustomObject]@{ id = "architecture-calibration"; suite = "evaluation\examples\opentopia-architecture-calibration-v1\suite.json"; target = "evaluation\examples\opentopia-architecture-calibration-v1\target.json" },
  [PSCustomObject]@{ id = "long-horizon"; suite = "evaluation\examples\opentopia-long-horizon-suite\suite.json"; target = "evaluation\examples\opentopia-long-horizon-suite\target.json" },
  [PSCustomObject]@{ id = "tool"; suite = "evaluation\examples\opentopia-tool-suite\suite.json"; target = "evaluation\examples\opentopia-tool-suite\target.json" },
  [PSCustomObject]@{ id = "long-horizon-new-5"; suite = "evaluation\examples\opentopia-long-horizon-new-5-suite\suite.json"; target = "evaluation\examples\opentopia-long-horizon-suite\target.json" },
  [PSCustomObject]@{ id = "multi-agent"; suite = "evaluation\examples\opentopia-multi-agent-suite\suite.json"; target = "evaluation\examples\opentopia-multi-agent-suite\target.json" }
)
$suites = if ($CalibrationOnly) { @($allSuites | Where-Object { $_.id -in @("architecture-calibration", "long-horizon") }) } else { $allSuites }
$taskCount = if ($CalibrationOnly) { 15 } else { 27 }
$experimentId = "deepseek-v4-flash-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$pairingKey = "internal-$taskCount-tasks-$Repetitions-repetitions"

if (-not $SkipBuild) {
  Build-EvaluationBinaries $beforeRootPath
  Build-EvaluationBinaries $afterRootPath
}

$manifest = [ordered]@{
  schemaVersion = 1
  experimentId = $experimentId
  startedAt = (Get-Date).ToUniversalTime().ToString("o")
  scope = if ($CalibrationOnly) { "unscored-calibration" } else { "scored-internal" }
  taskCount = $taskCount
  repetitions = $Repetitions
  executionOrder = "alternating before/after by repetition"
  provider = [ordered]@{
    model = $Model
    upstreamBaseUrl = $ProviderUpstreamBaseUrl.TrimEnd("/") + "/v1"
    reasoningEffort = $ReasoningEffort
    priceUsdPerMillionTokens = [ordered]@{
      input = $InputPricePerMillion
      cacheHitInput = $CacheHitPricePerMillion
      output = $OutputPricePerMillion
    }
  }
  snapshots = [ordered]@{
    before = Get-SnapshotEvidence $beforeRootPath
    after = Get-SnapshotEvidence $afterRootPath
  }
  suites = @($suites)
  runs = @()
}
$manifestPath = Join-Path $outputRootPath "experiment.json"
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 20) + "`n", [Text.UTF8Encoding]::new($false))

foreach ($repetition in 1..$Repetitions) {
  $variants = if ($repetition % 2 -eq 1) { @("before", "after") } else { @("after", "before") }
  foreach ($suite in $suites) {
    foreach ($variant in $variants) {
      $root = if ($variant -eq "before") { $beforeRootPath } else { $afterRootPath }
      $runId = "r{0:D2}-{1}-{2}" -f $repetition, $suite.id, $variant
      $runRoot = Join-Path $outputRootPath $runId
      New-Item -ItemType Directory -Path $runRoot -Force | Out-Null
      $usageLog = Join-Path $runRoot "provider-usage.jsonl"
      $usageSummary = Join-Path $runRoot "provider-usage-summary.json"
      $proxyStdout = Join-Path $runRoot "proxy.stdout.log"
      $proxyStderr = Join-Path $runRoot "proxy.stderr.log"
      $proxy = Start-Process -FilePath "node.exe" -ArgumentList @($proxyScript, "--port", $ProxyPort, "--upstream", $ProviderUpstreamBaseUrl.TrimEnd("/"), "--log", $usageLog) -RedirectStandardOutput $proxyStdout -RedirectStandardError $proxyStderr -WindowStyle Hidden -PassThru
      $deadline = (Get-Date).AddSeconds(10)
      while ((Get-Date) -lt $deadline -and -not (Test-TcpPort $ProxyPort)) { Start-Sleep -Milliseconds 100 }
      if (-not (Test-TcpPort $ProxyPort)) {
        $proxy | Stop-Process -Force -ErrorAction SilentlyContinue
        throw "provider telemetry proxy did not become reachable for $runId"
      }
      $summaryPath = Join-Path $runRoot "runner-summary.json"
      $runnerStdout = Join-Path $runRoot "runner.stdout.log"
      $runnerStderr = Join-Path $runRoot "runner.stderr.log"
      $runner = Join-Path $root $runnerRelativePath
      $arguments = @(
        "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", $runner,
        "-EnvFile", $envFilePath,
        "-Profile", "AUDIT_COPILOT_LLM",
        "-ExpectedModel", $Model,
        "-BaseUrlOverride", "http://127.0.0.1:$ProxyPort/v1",
        "-ReasoningEffort", $ReasoningEffort,
        "-Port", $ServerPort,
        "-Repetitions", "1",
        "-SuitePath", (Join-Path $root $suite.suite),
        "-TargetPath", (Join-Path $root $suite.target),
        "-SummaryPath", $summaryPath,
        "-OutputDirectory", (Join-Path $runRoot "harness"),
        "-ExperimentId", $experimentId,
        "-PairingKey", $pairingKey,
        "-Variant", $(if ($variant -eq "before") { "baseline" } else { "candidate" }),
        "-SkipBuild"
      )
      try {
        $process = Start-Process -FilePath "powershell.exe" -ArgumentList $arguments -RedirectStandardOutput $runnerStdout -RedirectStandardError $runnerStderr -WindowStyle Hidden -Wait -PassThru
        $exitCode = $process.ExitCode
      } finally {
        # Give the proxy's cloned response body a moment to append the final
        # streaming usage event before shutting down the local listener.
        Start-Sleep -Milliseconds 500
        $proxy | Stop-Process -Force -ErrorAction SilentlyContinue
        $proxy.WaitForExit(10000)
      }
      & node $usageSummaryScript --input $usageLog --output $usageSummary --input-price-per-million $InputPricePerMillion --cache-hit-price-per-million $CacheHitPricePerMillion --output-price-per-million $OutputPricePerMillion
      if ($LASTEXITCODE -ne 0) { throw "could not summarize provider usage for $runId" }
      $manifest.runs += [ordered]@{
        runId = $runId
        repetition = $repetition
        suite = $suite.id
        variant = $variant
        runnerExitCode = $exitCode
        summaryPath = $summaryPath
        providerUsageSummaryPath = $usageSummary
      }
      [IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 20) + "`n", [Text.UTF8Encoding]::new($false))
    }
  }
}

$manifest.completedAt = (Get-Date).ToUniversalTime().ToString("o")
[IO.File]::WriteAllText($manifestPath, ($manifest | ConvertTo-Json -Depth 20) + "`n", [Text.UTF8Encoding]::new($false))
Write-Output "PAIRED_EVALUATION_MANIFEST=$manifestPath"
