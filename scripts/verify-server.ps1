$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  cargo build -p opentopia-server

  $serverPath = Join-Path (Get-Location) "target\debug\opentopia-server.exe"
  if (-not (Test-Path $serverPath)) {
    $serverPath = Join-Path (Get-Location) "target\debug\opentopia-server"
  }
  if (-not (Test-Path $serverPath)) {
    throw "opentopia-server debug binary not found"
  }

  $dbPath = Join-Path (Get-Location) ".opentopia\verify.db"
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
        if ($health.ok -eq $true) {
          $healthy = $true
          break
        }
      } catch {
      }
    }

    if (-not $healthy) {
      throw "server did not become healthy"
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

    [PSCustomObject]@{
      healthy = $healthy
      threadId = $thread.id
      eventCount = $events.Count
    }
  } finally {
    if ($server -and -not $server.HasExited) {
      Stop-Process -Id $server.Id -Force
    }
  }
} finally {
  Pop-Location
}
