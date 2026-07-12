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
  if (-not (Test-Path $serverPath)) {
    $serverPath = Join-Path $env:CARGO_TARGET_DIR "debug\opentopia-server"
  }
  if (-not (Test-Path $serverPath)) {
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

    $threadBody = @{
      title = "verification"
      workspaceRoot = (Get-Location).Path
    } | ConvertTo-Json

    $thread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads" `
      -ContentType "application/json" `
      -Body $threadBody

    $messageBody = @{ content = "/list" } | ConvertTo-Json
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/messages" `
      -ContentType "application/json" `
      -Body $messageBody | Out-Null

    Start-Sleep -Seconds 1

    $events = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8799/api/threads/$($thread.id)/events" `
      -TimeoutSec 5

    Invoke-RestMethod `
      -Method Patch `
      -Uri "http://127.0.0.1:8799/api/settings" `
      -ContentType "application/json" `
      -Body (@{ permissionMode = "full_access" } | ConvertTo-Json) | Out-Null
    $cancelThread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads" `
      -ContentType "application/json" `
      -Body (@{ title = "cancel-verification"; workspaceRoot = (Get-Location).Path } | ConvertTo-Json)
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($cancelThread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = '/run powershell -NoProfile -Command "Start-Sleep -Seconds 30"' } | ConvertTo-Json) | Out-Null
    $activeTurn = $null
    for ($i = 0; $i -lt 30 -and -not $activeTurn; $i += 1) {
      Start-Sleep -Milliseconds 100
      $activeTurn = Invoke-RestMethod -Uri "http://127.0.0.1:8799/api/threads/$($cancelThread.id)/turn"
    }
    if (-not $activeTurn) {
      throw "expected a running agent turn"
    }
    try {
      Invoke-RestMethod `
        -Method Post `
        -Uri "http://127.0.0.1:8799/api/threads/$($cancelThread.id)/messages" `
        -ContentType "application/json" `
        -Body (@{ content = "/list" } | ConvertTo-Json) | Out-Null
      throw "concurrent message unexpectedly started"
    } catch {
      if ($_.Exception.Response.StatusCode.value__ -ne 409) {
        throw
      }
    }
    $cancelResult = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($cancelThread.id)/turn/cancel" `
      -ContentType "application/json" `
      -Body (@{ turnId = $activeTurn.turnId } | ConvertTo-Json)
    if (-not $cancelResult.cancelled) {
      throw "turn cancellation was not accepted"
    }
    $remainingTurn = $activeTurn
    for ($i = 0; $i -lt 100; $i += 1) {
      Start-Sleep -Milliseconds 100
      $remainingTurn = Invoke-RestMethod -Uri "http://127.0.0.1:8799/api/threads/$($cancelThread.id)/turn"
      if (-not $remainingTurn.turnId) {
        $remainingTurn = $null
        break
      }
    }
    if ($remainingTurn) {
      throw "cancelled turn remained active with status $($remainingTurn.status)"
    }

    Invoke-RestMethod `
      -Method Patch `
      -Uri "http://127.0.0.1:8799/api/settings" `
      -ContentType "application/json" `
      -Body (@{ permissionMode = "approve" } | ConvertTo-Json) | Out-Null
    $approvalWorkspace = Join-Path (Get-Location) ".opentopia\approval-verification"
    Remove-Item -LiteralPath $approvalWorkspace -Recurse -Force -ErrorAction SilentlyContinue
    New-Item -ItemType Directory -Path $approvalWorkspace -Force | Out-Null
    $approvalThread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads" `
      -ContentType "application/json" `
      -Body (@{ title = "approval-verification"; workspaceRoot = $approvalWorkspace } | ConvertTo-Json)
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($approvalThread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = "/write approved.txt`napproved through continuation" } | ConvertTo-Json) | Out-Null
    $pendingApproval = $null
    for ($i = 0; $i -lt 30 -and -not $pendingApproval; $i += 1) {
      Start-Sleep -Milliseconds 100
      $pending = @(Invoke-RestMethod -Uri "http://127.0.0.1:8799/api/threads/$($approvalThread.id)/approvals?status=pending")
      $pendingApproval = $pending | Select-Object -First 1
    }
    if (-not $pendingApproval) {
      throw "expected a persisted pending approval"
    }
    $approvalDecision = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8799/api/threads/$($approvalThread.id)/approvals/$($pendingApproval.approvalId)/decision" `
      -ContentType "application/json" `
      -Body (@{ approved = $true } | ConvertTo-Json)
    if (-not $approvalDecision.executed) {
      throw "approved continuation was not accepted for execution"
    }
    $approvedPath = Join-Path $approvalWorkspace "approved.txt"
    for ($i = 0; $i -lt 30 -and -not (Test-Path -LiteralPath $approvedPath); $i += 1) {
      Start-Sleep -Milliseconds 100
    }
    if ((Get-Content -LiteralPath $approvedPath -Raw) -ne "approved through continuation") {
      throw "approved continuation did not execute the exact suspended write"
    }

    [PSCustomObject]@{
      healthy = $healthy
      threadId = $thread.id
      eventCount = $events.Count
      unauthenticatedStatus = 401
      disallowedOriginStatus = 403
      concurrentTurnStatus = 409
      turnCancelled = $cancelResult.cancelled
      approvalResumed = $approvalDecision.executed
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
