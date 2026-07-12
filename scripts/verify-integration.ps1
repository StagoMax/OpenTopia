$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

$apiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }
$PSDefaultParameterValues["Invoke-RestMethod:Headers"] = $apiHeaders

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  $repoRoot = (Get-Location).Path
  $hunkWorkspace = Join-Path $repoRoot ".opentopia\integration-hunk-workspace"
  if (-not $hunkWorkspace.StartsWith((Join-Path $repoRoot ".opentopia"), [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "refusing to prepare hunk workspace outside .opentopia"
  }
  Remove-Item -LiteralPath $hunkWorkspace -Recurse -Force -ErrorAction SilentlyContinue
  New-Item -ItemType Directory -Path $hunkWorkspace -Force | Out-Null
  git -C $hunkWorkspace init --quiet
  git -C $hunkWorkspace config user.email "integration@opentopia.local"
  git -C $hunkWorkspace config user.name "OpenTopia Integration"
  1..24 | ForEach-Object { "line $_" } | Set-Content -LiteralPath (Join-Path $hunkWorkspace "sample.txt")
  git -C $hunkWorkspace add sample.txt
  git -C $hunkWorkspace commit --quiet -m "initial"
  $changedLines = 1..24 | ForEach-Object {
    if ($_ -eq 2 -or $_ -eq 22) { "changed line $_" } else { "line $_" }
  }
  $changedLines | Set-Content -LiteralPath (Join-Path $hunkWorkspace "sample.txt")

  $targetDir = Join-Path (Get-Location) ".opentopia\verify-target"
  $previousCargoTargetDir = $env:CARGO_TARGET_DIR
  $env:CARGO_TARGET_DIR = $targetDir
  cargo build -p opentopia-server

  $serverPath = Join-Path $targetDir "debug\opentopia-server.exe"
  if (-not (Test-Path $serverPath)) {
    $serverPath = Join-Path $targetDir "debug\opentopia-server"
  }
  if (-not (Test-Path $serverPath)) {
    throw "opentopia-server debug binary not found"
  }

  $dbPath = Join-Path (Get-Location) ".opentopia\integration-smoke.db"
  Remove-Item -LiteralPath $dbPath, "$dbPath-shm", "$dbPath-wal" -Force -ErrorAction SilentlyContinue

  $server = Start-Process `
    -FilePath $serverPath `
    -ArgumentList @("--port", "8801", "--db", $dbPath, "--permission", "auto") `
    -PassThru `
    -WindowStyle Hidden

  try {
    $healthy = $false
    for ($i = 0; $i -lt 30; $i += 1) {
      Start-Sleep -Milliseconds 300
      try {
        $health = Invoke-RestMethod -Uri "http://127.0.0.1:8801/health" -TimeoutSec 2
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
      throw "integration server did not become healthy"
    }

    $thread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads" `
      -ContentType "application/json" `
      -Body (@{ title = "integration"; workspaceRoot = (Get-Location).Path } | ConvertTo-Json)

    $settings = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/settings" `
      -TimeoutSec 5
    $updatedSettings = Invoke-RestMethod `
      -Method Patch `
      -Uri "http://127.0.0.1:8801/api/settings" `
      -ContentType "application/json" `
      -Body (@{
        model = "opentopia-integration-model"
        permissionMode = "auto"
      } | ConvertTo-Json)
    $activeProvider = @($updatedSettings.providers | Where-Object {
      $_.id -eq $updatedSettings.activeProviderId
    })[0]
    if (-not $activeProvider) {
      $activeProvider = @($updatedSettings.providers)[0]
    }
    if ($activeProvider.model -ne "opentopia-integration-model") {
      throw "settings update did not persist model"
    }

    $tree = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/workspace/tree" `
      -TimeoutSec 5
    if (@($tree.entries).Count -lt 1) {
      throw "expected workspace tree entries"
    }

    $diff = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/workspace/diff" `
      -TimeoutSec 5

    $hunkThread = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads" `
      -ContentType "application/json" `
      -Body (@{ title = "hunk-integration"; workspaceRoot = $hunkWorkspace } | ConvertTo-Json)
    $hunkDiff = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($hunkThread.id)/workspace/diff" `
      -TimeoutSec 5
    $unstagedHunks = @($hunkDiff.hunks | Where-Object { $_.scope -eq "unstaged" })
    if ($unstagedHunks.Count -lt 2) {
      throw "expected two independent unstaged hunks"
    }

    $stagedResult = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($hunkThread.id)/workspace/diff/hunk" `
      -ContentType "application/json" `
      -Body (@{
        path = $unstagedHunks[0].path
        scope = $unstagedHunks[0].scope
        patch = $unstagedHunks[0].patch
        action = "stage"
        confirm = $true
      } | ConvertTo-Json -Depth 8)
    if (@($stagedResult.diff.hunks | Where-Object { $_.scope -eq "staged" }).Count -lt 1) {
      throw "expected staged hunk after stage action"
    }

    $stagedHunk = @($stagedResult.diff.hunks | Where-Object { $_.scope -eq "staged" })[0]
    $unstagedResult = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($hunkThread.id)/workspace/diff/hunk" `
      -ContentType "application/json" `
      -Body (@{
        path = $stagedHunk.path
        scope = $stagedHunk.scope
        patch = $stagedHunk.patch
        action = "unstage"
        confirm = $true
      } | ConvertTo-Json -Depth 8)
    $discardHunk = @($unstagedResult.diff.hunks | Where-Object { $_.scope -eq "unstaged" })[0]
    $discardedResult = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($hunkThread.id)/workspace/diff/hunk" `
      -ContentType "application/json" `
      -Body (@{
        path = $discardHunk.path
        scope = $discardHunk.scope
        patch = $discardHunk.patch
        action = "discard"
        confirm = $true
      } | ConvertTo-Json -Depth 8)
    if (@($discardedResult.diff.hunks).Count -ge @($unstagedResult.diff.hunks).Count) {
      throw "expected discard action to remove one hunk"
    }

    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/commands" `
      -ContentType "application/json" `
      -Body (@{ command = "echo terminal-persisted" } | ConvertTo-Json) | Out-Null
    Start-Sleep -Seconds 1
    $terminalHistory = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/history" `
      -TimeoutSec 5
    if (@($terminalHistory | Where-Object { $_.data -match "terminal-persisted" }).Count -lt 1) {
      throw "expected terminal output in persistent history"
    }

    $pty = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session" `
      -ContentType "application/json" `
      -Body (@{ cols = 100; rows = 24 } | ConvertTo-Json)
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session/resize" `
      -ContentType "application/json" `
      -Body (@{ sessionId = $pty.sessionId; cols = 120; rows = 32 } | ConvertTo-Json) | Out-Null
    if ($env:OS -eq "Windows_NT") {
      Start-Sleep -Milliseconds 250
      Invoke-RestMethod `
        -Method Post `
        -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session/input" `
        -ContentType "application/json" `
        -Body (@{
          sessionId = $pty.sessionId
          data = "$([char]27)[1;1R"
        } | ConvertTo-Json) | Out-Null
    }
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session/input" `
      -ContentType "application/json" `
      -Body (@{ sessionId = $pty.sessionId; data = "echo pty-persisted`r" } | ConvertTo-Json) | Out-Null
    $ptyOutputSeen = $false
    for ($i = 0; $i -lt 20; $i += 1) {
      Start-Sleep -Milliseconds 250
      $ptyEvents = Invoke-RestMethod `
        -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/history" `
        -TimeoutSec 5
      $ptyOutputSeen = @($ptyEvents | Where-Object {
        $_.commandId -eq $pty.sessionId -and $_.data -match "pty-persisted"
      }).Count -gt 0
      if ($ptyOutputSeen) { break }
    }
    if (-not $ptyOutputSeen) {
      throw "expected output from persistent PTY session"
    }
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session/close" `
      -ContentType "application/json" `
      -Body (@{ sessionId = $pty.sessionId } | ConvertTo-Json) | Out-Null
    $ptyClosed = $false
    for ($i = 0; $i -lt 30; $i += 1) {
      Start-Sleep -Milliseconds 200
      $activePty = Invoke-RestMethod `
        -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/session" `
        -TimeoutSec 5
      if ($null -eq $activePty -or $activePty.status -ne "running") {
        $ptyClosed = $true
        break
      }
    }
    if (-not $ptyClosed) {
      throw "expected persistent PTY session to close"
    }
    $closedPtyHistory = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/terminal/history" `
      -TimeoutSec 5
    if (@($closedPtyHistory | Where-Object {
      $_.commandId -eq $pty.sessionId -and $_.data -match "pty-persisted"
    }).Count -lt 1) {
      throw "expected closed PTY output in persistent history"
    }
    if (@($closedPtyHistory | Where-Object {
      $_.commandId -eq $pty.sessionId -and $_.type -eq "cancelled"
    }).Count -ne 1) {
      throw "expected one persisted PTY cancellation event"
    }

    $sandbox = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/sandbox" `
      -TimeoutSec 5
    if ($sandbox.kind -ne "local") {
      throw "expected local sandbox descriptor"
    }

    $mcp = Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/mcp/servers" `
      -ContentType "application/json" `
      -Body (@{
        name = "integration-echo"
        command = "node"
        args = @("--version")
        envKeys = @("PATH")
      } | ConvertTo-Json)
    Invoke-RestMethod `
      -Method Put `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/mcp/$($mcp.server.serverId)" `
      -ContentType "application/json" `
      -Body (@{ enabled = $true } | ConvertTo-Json) | Out-Null
    $threadMcp = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/mcp" `
      -TimeoutSec 5
    if (@($threadMcp | Where-Object { $_.server.serverId -eq $mcp.server.serverId -and $_.enabled }).Count -ne 1) {
      throw "expected enabled thread MCP binding"
    }

    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = "/search AgentCore" } | ConvertTo-Json) | Out-Null

    $hasSearchFinish = $false
    for ($i = 0; $i -lt 20; $i += 1) {
      Start-Sleep -Milliseconds 500
      $searchEvents = Invoke-RestMethod `
        -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/events" `
        -TimeoutSec 5
      $hasSearchFinish = @(
        $searchEvents | Where-Object {
          $_.payload.type -eq "tool_call_finished" -and $_.payload.result.output -match "AgentCore"
        }
      ).Count -gt 0
      if ($hasSearchFinish) { break }
    }
    if (-not $hasSearchFinish) {
      throw "expected sandboxed search tool to finish"
    }

    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/messages" `
      -ContentType "application/json" `
      -Body (@{ content = "/run git reset --hard" } | ConvertTo-Json) | Out-Null

    Start-Sleep -Seconds 1
    $pending = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/approvals?status=pending" `
      -TimeoutSec 5
    if (@($pending).Count -lt 1) {
      throw "expected at least one pending approval"
    }

    $approvalId = @($pending)[0].approvalId
    Invoke-RestMethod `
      -Method Post `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/approvals/$approvalId/decision" `
      -ContentType "application/json" `
      -Body (@{ approved = $false } | ConvertTo-Json) | Out-Null

    $denied = Invoke-RestMethod `
      -Uri "http://127.0.0.1:8801/api/threads/$($thread.id)/approvals?status=denied" `
      -TimeoutSec 5

    [PSCustomObject]@{
      healthy = $healthy
      threadId = $thread.id
      settingsLoaded = $null -ne $settings
      settingsUpdated = $activeProvider.model
      workspaceEntries = @($tree.entries).Count
      changedFiles = @($diff.files).Count
      sandboxKind = $sandbox.kind
      mcpServers = @($threadMcp).Count
      searchToolFinished = $hasSearchFinish
      pendingBeforeDeny = @($pending).Count
      deniedAfterDeny = @($denied).Count
      hunkStageUnstageDiscard = $true
      terminalHistoryPersisted = $true
      persistentPty = $ptyOutputSeen
      persistentPtyClosed = $ptyClosed
    }
  } finally {
    if ($server -and -not $server.HasExited) {
      Stop-Process -Id $server.Id -Force
    }
  }
} finally {
  if ($hunkWorkspace -and (Test-Path $hunkWorkspace)) {
    $allowedRoot = Join-Path $repoRoot ".opentopia"
    if ($hunkWorkspace.StartsWith($allowedRoot, [System.StringComparison]::OrdinalIgnoreCase)) {
      Remove-Item -LiteralPath $hunkWorkspace -Recurse -Force
    }
  }
  if ($null -eq $previousCargoTargetDir) {
    Remove-Item Env:\CARGO_TARGET_DIR -ErrorAction SilentlyContinue
  } else {
    $env:CARGO_TARGET_DIR = $previousCargoTargetDir
  }
  Pop-Location
}
