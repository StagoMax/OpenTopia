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
$sandboxBinaryName = if ($isWindowsHost) { "opentopia-sandbox.exe" } else { "opentopia-sandbox" }
$releaseSandboxBinary = Join-Path $repoRoot "target\release\$sandboxBinaryName"
$sandboxResources = Join-Path $resourcesDir "opentopia-sandbox"
$resourceSandboxBinary = Join-Path $sandboxResources $sandboxBinaryName

Push-Location $repoRoot
try {
  Write-Host "Building Rust server and Windows sandbox: cargo build --release -p opentopia-server -p opentopia-windows-sandbox"
  cargo build --release -p opentopia-server -p opentopia-windows-sandbox
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
    if (-not (Test-Path -LiteralPath $releaseSandboxBinary)) {
      throw "opentopia-sandbox release binary not found at $releaseSandboxBinary"
    }
    New-Item -ItemType Directory -Force -Path $sandboxResources | Out-Null
    Copy-Item -LiteralPath $releaseSandboxBinary -Destination $resourceSandboxBinary -Force
    if (-not (Test-Path -LiteralPath $resourceSandboxBinary)) {
      throw "Failed to stage OpenTopia Windows sandbox at $resourceSandboxBinary"
    }
    Write-Host "Staged first-party Windows sandbox for electron-builder: $resourceSandboxBinary"
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
