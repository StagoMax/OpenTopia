$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

$apiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }
$PSDefaultParameterValues["Invoke-RestMethod:Headers"] = $apiHeaders

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  if ([string]::IsNullOrWhiteSpace($env:OPENTOPIA_API_KEY)) {
    throw "OPENTOPIA_API_KEY is not configured; set OPENTOPIA_ENV_FILE or provide a supported API key alias"
  }

  $targetDir = Join-Path (Get-Location) ".opentopia\verify-target"
  $previousCargoTargetDir = $env:CARGO_TARGET_DIR
  $env:CARGO_TARGET_DIR = $targetDir
  cargo build -p opentopia-server

  $serverPath = Join-Path $targetDir "debug\opentopia-server.exe"
  if (-not (Test-Path -LiteralPath $serverPath)) {
    $serverPath = Join-Path $targetDir "debug\opentopia-server"
  }
  if (-not (Test-Path -LiteralPath $serverPath)) {
    throw "opentopia-server debug binary not found"
  }

  $dbPath = Join-Path (Get-Location) ".opentopia\context-summary-smoke.db"
  Remove-Item -LiteralPath $dbPath, "$dbPath-shm", "$dbPath-wal" -Force -ErrorAction SilentlyContinue
  $server = Start-Process `
    -FilePath $serverPath `
    -ArgumentList @("--port", "8802", "--db", $dbPath, "--permission", "auto") `
    -PassThru `
    -WindowStyle Hidden

  try {
    $healthy = $false
    for ($i = 0; $i -lt 30; $i += 1) {
      Start-Sleep -Milliseconds 300
      try {
        $health = Invoke-RestMethod -Uri "http://127.0.0.1:8802/health" -TimeoutSec 2
        if (
          $health.ok -and
          $health.service -eq "opentopia-server" -and
          $health.apiVersion -eq 1
        ) {
          $healthy = $true
          break
        }
      } catch {
      }
    }
    if (-not $healthy) {
      throw "context summary server did not become healthy"
    }

    $settings = Invoke-RestMethod -Uri "http://127.0.0.1:8802/api/settings" -TimeoutSec 5
    $provider = @($settings.providers | Where-Object {
      $_.id -eq $settings.activeProviderId
    })[0]
    if (-not $provider) {
      $provider = @($settings.providers)[0]
    }
    if (-not $provider.apiKeyConfigured -or $provider.kind -ne "open_ai_compatible") {
      throw "active provider is not configured for real context summarization"
    }

    $thread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8802/api/threads" `
      -ContentType "application/json" `
      -Body (@{
        title = "real context summary smoke"
        workspaceRoot = (Get-Location).Path
      } | ConvertTo-Json)

    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8802/api/threads/$($thread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = "/search ContextSummary" } | ConvertTo-Json) | Out-Null

    $trajectoryReady = $false
    for ($i = 0; $i -lt 30; $i += 1) {
      Start-Sleep -Milliseconds 500
      $events = Invoke-RestMethod `
        -Uri "http://127.0.0.1:8802/api/threads/$($thread.id)/events" `
        -TimeoutSec 5
      $trajectoryReady = @($events | Where-Object {
        $_.payload.type -eq "tool_call_finished" -and $_.payload.result.output -match "ContextSummary"
      }).Count -gt 0
      if ($trajectoryReady) {
        break
      }
    }
    if (-not $trajectoryReady) {
      throw "failed to prepare context trajectory before summarization"
    }

    $summary = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8802/api/threads/$($thread.id)/context/compact" `
      -ContentType "application/json" `
      -Body "{}" `
      -TimeoutSec 100
    if ($summary.metadata.mode -ne "llm") {
      throw "expected LLM context compaction mode"
    }
    if ([string]::IsNullOrWhiteSpace($summary.summary)) {
      throw "real context summary was empty"
    }
    if ($summary.metadata.providerId -ne $provider.id -or $summary.metadata.model -ne $provider.model) {
      throw "context summary provider metadata does not match active settings"
    }

    $status = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8802/api/threads/$($thread.id)/context" `
      -TimeoutSec 5
    if ($status.latestSummary.id -ne $summary.id) {
      throw "context summary was not persisted as the latest durable summary"
    }

    [PSCustomObject]@{
      healthy = $healthy
      threadId = $thread.id
      providerId = $provider.id
      model = $provider.model
      mode = $summary.metadata.mode
      coveredMessages = $summary.messageCount
      coveredThroughSeq = $summary.coveredThroughSeq
      summaryCharacters = $summary.summary.Length
      persisted = $status.latestSummary.id -eq $summary.id
    }
  } finally {
    if ($server -and -not $server.HasExited) {
      Stop-Process -Id $server.Id -Force
    }
  }
} finally {
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  Pop-Location
}
