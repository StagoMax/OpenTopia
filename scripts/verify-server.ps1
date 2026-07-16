$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

$apiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }
$PSDefaultParameterValues["Invoke-RestMethod:Headers"] = $apiHeaders

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  $previousCargoTargetDir = $env:CARGO_TARGET_DIR
  $env:CARGO_TARGET_DIR = Join-Path (Get-Location) ".opentopia\verify-target"
  cargo build -p opentopia-server

  $serverPath = Join-Path $env:CARGO_TARGET_DIR "debug\opentopia-server.exe"
  if (-not (Test-Path -LiteralPath $serverPath)) {
    $serverPath = Join-Path $env:CARGO_TARGET_DIR "debug\opentopia-server"
  }
  if (-not (Test-Path -LiteralPath $serverPath)) {
    throw "opentopia-server debug binary not found"
  }

  $dbPath = Join-Path (Get-Location) ".opentopia\verify.db"
  Remove-Item -LiteralPath $dbPath, "$dbPath-shm", "$dbPath-wal" -Force -ErrorAction SilentlyContinue
  $server = Start-Process `
    -FilePath $serverPath `
    -ArgumentList @("--port", "8799", "--db", $dbPath, "--permission", "auto") `
    -PassThru `
    -WindowStyle Hidden

  try {
    $healthy = $false
    for ($i = 0; $i -lt 30; $i += 1) {
      Start-Sleep -Milliseconds 300
      try {
        $health = Invoke-RestMethod -Uri "http://127.0.0.1:8799/health" -TimeoutSec 2
        if (
          $health.ok -eq $true -and
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
      throw "server did not become healthy"
    }

    try {
      Invoke-WebRequest -Uri "http://127.0.0.1:8799/health" -TimeoutSec 2 | Out-Null
      throw "health unexpectedly accepted a request without a bearer token"
    } catch {
      if ($_.Exception.Response.StatusCode.value__ -ne 401) {
        throw
      }
    }

    try {
      Invoke-WebRequest `
        -Uri "http://127.0.0.1:8799/health" `
        -Headers ($apiHeaders + @{ Origin = "https://attacker.example" }) `
        -TimeoutSec 2 | Out-Null
      throw "health unexpectedly accepted a disallowed browser origin"
    } catch {
      if ($_.Exception.Response.StatusCode.value__ -ne 403) {
        throw
      }
    }

    Invoke-RestMethod `
      -Method Patch `
      -Uri "http://127.0.0.1:8799/api/settings" `
      -ContentType "application/json" `
      -Body (@{ providerKind = "mock"; permissionMode = "auto" } | ConvertTo-Json) | Out-Null

    $thread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads" `
      -ContentType "application/json" `
      -Body (@{
        title = "verification"
        workspaceRoot = (Get-Location).Path
      } | ConvertTo-Json)

    $preview = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/previews/resolve" `
      -ContentType "application/json" `
      -Body (@{ source = "workspace"; path = "Cargo.toml" } | ConvertTo-Json)
    if ($preview.kind -ne "text" -or $preview.contentType -notlike "text/*") {
      throw "expected Cargo.toml to resolve as a text preview"
    }

    $previewContent = Invoke-WebRequest `
      -UseBasicParsing `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/previews/$([uri]::EscapeDataString($preview.id))/content" `
      -Headers $apiHeaders `
      -TimeoutSec 5
    if ($previewContent.Content -notmatch "\[workspace\]") {
      throw "preview content did not preserve Cargo.toml text"
    }
    if ($previewContent.Headers["X-Content-Type-Options"] -ne "nosniff") {
      throw "preview response is missing nosniff"
    }

    try {
      Invoke-WebRequest `
        -UseBasicParsing `
        -Method Post `
        -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/previews/resolve" `
        -Headers $apiHeaders `
        -ContentType "application/json" `
        -Body (@{ source = "workspace"; path = "..\Cargo.toml" } | ConvertTo-Json) | Out-Null
      throw "preview unexpectedly accepted a parent-directory escape"
    } catch {
      if ($_.Exception.Response.StatusCode.value__ -ne 400) {
        throw
      }
    }

    $message = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = "Return a concise verification response." } | ConvertTo-Json)

    $turn = $null
    for ($i = 0; $i -lt 100; $i += 1) {
      Start-Sleep -Milliseconds 100
      $turn = Invoke-RestMethod -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/turn"
      if ($turn.status -in @("succeeded", "failed", "cancelled", "interrupted")) {
        break
      }
    }
    if (-not $turn) {
      throw "expected a persisted turn record"
    }
    if ($turn.status -ne "succeeded") {
      throw "expected succeeded turn, found $($turn.status): $($turn.error)"
    }
    if ($turn.userMessageId -ne $message.id) {
      throw "turn does not reference the submitted user message"
    }

    $events = @(Invoke-RestMethod `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/events" `
      -TimeoutSec 5)
    $eventTypes = @($events | ForEach-Object { $_.payload.type })
    foreach ($requiredType in @("turn_started", "assistant_message", "turn_finished")) {
      if ($requiredType -notin $eventTypes) {
        throw "missing required event type: $requiredType"
      }
    }

    try {
      Invoke-WebRequest `
        -Method Post `
        -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/messages" `
        -Headers $apiHeaders `
        -ContentType "application/json" `
        -Body (@{ content = "/run echo bypass" } | ConvertTo-Json) | Out-Null
      throw "legacy direct command unexpectedly reached the agent"
    } catch {
      if ($_.Exception.Response.StatusCode.value__ -ne 400) {
        throw
      }
    }

    [PSCustomObject]@{
      healthy = $healthy
      threadId = $thread.id
      turnId = $turn.turnId
      turnStatus = $turn.status
      eventCount = $eventTypes.Count
      previewKind = $preview.kind
      previewBytes = $previewContent.RawContentLength
      previewEscapeStatus = 400
      unauthenticatedStatus = 401
      disallowedOriginStatus = 403
      legacyCommandStatus = 400
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
