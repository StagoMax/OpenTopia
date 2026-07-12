$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

function Invoke-Pnpm {
  param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)

  if (Get-Command corepack.cmd -ErrorAction SilentlyContinue) {
    & corepack.cmd pnpm @Arguments
    if ($LASTEXITCODE -ne 0) {
      throw "pnpm failed with exit code $LASTEXITCODE"
    }
    return
  }

  if (Get-Command pnpm.cmd -ErrorAction SilentlyContinue) {
    & pnpm.cmd @Arguments
    if ($LASTEXITCODE -ne 0) {
      throw "pnpm failed with exit code $LASTEXITCODE"
    }
    return
  }

  throw "pnpm was not found. Install pnpm or enable Corepack for Node.js."
}

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$desktopRoot = Join-Path $repoRoot "apps\desktop"
$resourcesDir = Join-Path $desktopRoot "resources"
$isWindowsHost = [System.Environment]::OSVersion.Platform -eq "Win32NT"
$serverBinaryName = if ($isWindowsHost) { "opentopia-server.exe" } else { "opentopia-server" }
$releaseServerBinary = Join-Path $repoRoot "target\release\$serverBinaryName"
$resourceServerBinary = Join-Path $resourcesDir $serverBinaryName
$codexSandboxResources = Join-Path $resourcesDir "codex-sandbox"

Push-Location $repoRoot
try {
  Write-Host "Building Rust server: cargo build --release -p opentopia-server"
  cargo build --release -p opentopia-server
  if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
  }

  if (-not (Test-Path -LiteralPath $releaseServerBinary)) {
    throw "opentopia-server release binary not found at $releaseServerBinary"
  }

  New-Item -ItemType Directory -Force -Path $resourcesDir | Out-Null
  Copy-Item -LiteralPath $releaseServerBinary -Destination $resourceServerBinary -Force

  if (-not (Test-Path -LiteralPath $resourceServerBinary)) {
    throw "Failed to stage server binary at $resourceServerBinary"
  }

  Write-Host "Staged server binary for electron-builder extraResources: $resourceServerBinary"

  if ($isWindowsHost) {
    $codexSandboxSource = if ($env:OPENTOPIA_CODEX_SANDBOX_DIR) {
      $env:OPENTOPIA_CODEX_SANDBOX_DIR
    } else {
      Join-Path $env:USERPROFILE ".codex\plugins\.plugin-appserver"
    }
    $requiredHelpers = @(
      "codex.exe",
      "codex-command-runner.exe",
      "codex-windows-sandbox-setup.exe"
    )
    foreach ($helper in $requiredHelpers) {
      $source = Join-Path $codexSandboxSource $helper
      if (-not (Test-Path -LiteralPath $source)) {
        throw "Required Codex Windows sandbox helper not found: $source"
      }
    }
    New-Item -ItemType Directory -Force -Path $codexSandboxResources | Out-Null
    foreach ($helper in $requiredHelpers) {
      Copy-Item `
        -LiteralPath (Join-Path $codexSandboxSource $helper) `
        -Destination (Join-Path $codexSandboxResources $helper) `
        -Force
    }
    $codexLicenseCandidates = @()
    if ($env:OPENTOPIA_CODEX_SOURCE) {
      $codexLicenseCandidates += Join-Path $env:OPENTOPIA_CODEX_SOURCE "LICENSE"
    }
    $codexLicenseCandidates += "J:\Project\codex cli\codex\LICENSE"
    $codexLicenseCandidates = @($codexLicenseCandidates | Where-Object {
      Test-Path -LiteralPath $_
    })
    if (@($codexLicenseCandidates).Count -lt 1) {
      throw "Codex Apache-2.0 LICENSE was not found; set OPENTOPIA_CODEX_SOURCE"
    }
    Copy-Item `
      -LiteralPath @($codexLicenseCandidates)[0] `
      -Destination (Join-Path $codexSandboxResources "LICENSE") `
      -Force
    Write-Host "Staged Codex restricted-token sandbox helpers: $codexSandboxResources"
  }
    Invoke-Pnpm --filter @opentopia/desktop build
    $electronBuilderArgs = @(
      "--filter",
      "@opentopia/desktop",
      "exec",
      "electron-builder"
    )
    if ($env:OPENTOPIA_ELECTRON_DIST) {
      $electronDist = (Resolve-Path -LiteralPath $env:OPENTOPIA_ELECTRON_DIST).Path
      $electronBuilderArgs += "--config.electronDist=$electronDist"
    }
    if ($env:OPENTOPIA_DESKTOP_OUTPUT_DIR) {
      $electronBuilderArgs += "--config.directories.output=$($env:OPENTOPIA_DESKTOP_OUTPUT_DIR)"
    }
    if ($env:OPENTOPIA_DISABLE_ASAR_INTEGRITY -eq "true") {
      $electronBuilderArgs += "--config.disableAsarIntegrity=true"
    }
    if ($env:OPENTOPIA_SKIP_EXE_EDIT -eq "true") {
      $electronBuilderArgs += "--config.win.signAndEditExecutable=false"
    }
    Invoke-Pnpm @electronBuilderArgs
} finally {
  Pop-Location
}
