param(
  [switch]$StageOnly,
  [string]$OfficeRuntimeSource,
  [string]$OfficePythonArchive,
  [string]$OfficePython,
  [string]$AgentToolsRuntimeSource,
  [string]$AgentToolsRipgrepArchive,
  [string]$AgentToolsGitArchive
)

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
$runtimeStageDir = Join-Path $desktopRoot ".runtime-stage"
$runtimeTempDir = "$runtimeStageDir.tmp-$PID"
$isWindowsHost = [System.Environment]::OSVersion.Platform -eq "Win32NT"
$serverBinaryName = if ($isWindowsHost) { "opentopia-server.exe" } else { "opentopia-server" }
$releaseServerBinary = Join-Path $repoRoot "target\release\$serverBinaryName"
$stagedServerBinary = Join-Path $runtimeTempDir $serverBinaryName
$sandboxBinaryName = if ($isWindowsHost) { "opentopia-sandbox.exe" } else { "opentopia-sandbox" }
$releaseSandboxBinary = Join-Path $repoRoot "target\release\$sandboxBinaryName"
$sandboxStageDir = Join-Path $runtimeTempDir "opentopia-sandbox"
$stagedSandboxBinary = Join-Path $sandboxStageDir $sandboxBinaryName
$runtimeManifestPath = Join-Path $runtimeTempDir "opentopia-runtime-manifest.json"
$officeRuntimeStageDir = Join-Path $runtimeTempDir "office-runtime"
$agentToolsRuntimeStageDir = Join-Path $runtimeTempDir "agent-tools"

function Assert-RuntimeStagePath {
  param([Parameter(Mandatory = $true)][string]$Path)
  $fullPath = [System.IO.Path]::GetFullPath($Path)
  $desktopPrefix = [System.IO.Path]::GetFullPath($desktopRoot) + [System.IO.Path]::DirectorySeparatorChar
  if (-not $fullPath.StartsWith($desktopPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Refusing to modify runtime stage outside the desktop directory: $fullPath"
  }
  if (-not ([System.IO.Path]::GetFileName($fullPath)).StartsWith(".runtime-stage")) {
    throw "Refusing to modify an unexpected runtime stage path: $fullPath"
  }
}

function Resolve-OfficeRuntimeSource {
  if ($OfficeRuntimeSource) {
    return (Resolve-Path -LiteralPath $OfficeRuntimeSource).Path
  }
  if ($env:OPENTOPIA_OFFICE_RUNTIME_SOURCE) {
    return (Resolve-Path -LiteralPath $env:OPENTOPIA_OFFICE_RUNTIME_SOURCE).Path
  }
  if ($OfficePython -or $env:OPENTOPIA_OFFICE_RUNTIME_PYTHON) {
    throw "Host Python/venv packaging is no longer supported. Use -OfficePythonArchive, OPENTOPIA_OFFICE_RUNTIME_ARCHIVE, or a prepared -OfficeRuntimeSource."
  }
  $default = Join-Path $repoRoot "runtime\office\dist"
  if (Test-Path -LiteralPath $default) {
    return (Resolve-Path -LiteralPath $default).Path
  }
  $lock = Get-Content -LiteralPath (Join-Path $repoRoot "runtime\office\runtime-lock.json") -Raw | ConvertFrom-Json
  $architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
  $arch = if ($architecture -eq "x64") { "x86_64" } elseif ($architecture -eq "arm64") { "aarch64" } else {
    throw "Managed Office Python is unsupported on architecture $architecture"
  }
  $isMacHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::OSX
  )
  $os = if ($isWindowsHost) { "windows" } elseif ($isMacHost) { "macos" } else { "linux" }
  $targetId = "$os-$arch"
  $prepared = Join-Path $repoRoot "runtime\office\cache\$($lock.runtimeVersion)\$targetId"
  $archive = if ($OfficePythonArchive) {
    $OfficePythonArchive
  } elseif ($env:OPENTOPIA_OFFICE_RUNTIME_ARCHIVE) {
    $env:OPENTOPIA_OFFICE_RUNTIME_ARCHIVE
  } else {
    $null
  }
  $prepareArgs = @("-Output", $prepared)
  if ($archive) { $prepareArgs += @("-PythonArchive", $archive) }
  & (Join-Path $PSScriptRoot "prepare-office-runtime.ps1") @prepareArgs
  return (Resolve-Path -LiteralPath $prepared).Path
}

function Assert-OfficeRuntimeSource {
  param([Parameter(Mandatory = $true)][string]$Path)
  $manifestPath = Join-Path $Path "office-runtime.json"
  if (-not (Test-Path -LiteralPath $manifestPath)) {
    throw "Office runtime manifest not found: $manifestPath"
  }
  $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  if ($manifest.schemaVersion -ne 2 -or $manifest.id -ne "ai.opentopia.office-runtime") {
    throw "Office runtime manifest has an unsupported identity: $manifestPath"
  }
  if (-not $manifest.python.path -or -not $manifest.python.sha256 -or
      -not $manifest.python.version -or -not $manifest.python.distribution -or
      -not $manifest.packages.openpyxl -or -not $manifest.packages.etXmlfile) {
    throw "Office runtime manifest has no Python artifact: $manifestPath"
  }
  $pythonPath = Join-Path $Path $manifest.python.path
  $runtimePrefix = [System.IO.Path]::GetFullPath($Path) + [System.IO.Path]::DirectorySeparatorChar
  $resolvedPython = [System.IO.Path]::GetFullPath($pythonPath)
  if (-not $resolvedPython.StartsWith($runtimePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Office runtime Python path escapes its runtime root"
  }
  if (-not (Test-Path -LiteralPath $resolvedPython)) {
    throw "Office runtime Python executable not found: $resolvedPython"
  }
  $actual = (Get-FileHash -LiteralPath $resolvedPython -Algorithm SHA256).Hash.ToLowerInvariant()
  if ($actual -ne $manifest.python.sha256.ToLowerInvariant()) {
    throw "Office runtime Python hash does not match its manifest"
  }
}

function Get-AgentToolsTargetId {
  $architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
  $arch = if ($architecture -eq "x64") {
    "x86_64"
  } elseif ($architecture -eq "arm64") {
    "aarch64"
  } else {
    throw "Agent tools runtime is unsupported on architecture $architecture"
  }
  return "windows-$arch"
}

function Resolve-AgentToolsRuntimeSource {
  if (-not $isWindowsHost) { return $null }
  if ($AgentToolsRuntimeSource) {
    return (Resolve-Path -LiteralPath $AgentToolsRuntimeSource).Path
  }
  if ($env:OPENTOPIA_AGENT_TOOLS_SOURCE) {
    return (Resolve-Path -LiteralPath $env:OPENTOPIA_AGENT_TOOLS_SOURCE).Path
  }

  $default = Join-Path $repoRoot "runtime\agent-tools\dist"
  if (Test-Path -LiteralPath (Join-Path $default "agent-tools-runtime.json")) {
    return (Resolve-Path -LiteralPath $default).Path
  }

  $lock = Get-Content -LiteralPath (Join-Path $repoRoot "runtime\agent-tools\runtime-lock.json") -Raw | ConvertFrom-Json
  $targetId = Get-AgentToolsTargetId
  $prepared = Join-Path $repoRoot "runtime\agent-tools\cache\$($lock.runtimeVersion)\$targetId"
  $prepareArgs = @("-Output", $prepared)
  $ripgrepArchive = if ($AgentToolsRipgrepArchive) {
    $AgentToolsRipgrepArchive
  } elseif ($env:OPENTOPIA_AGENT_TOOLS_RG_ARCHIVE) {
    $env:OPENTOPIA_AGENT_TOOLS_RG_ARCHIVE
  } else {
    $null
  }
  $gitArchive = if ($AgentToolsGitArchive) {
    $AgentToolsGitArchive
  } elseif ($env:OPENTOPIA_AGENT_TOOLS_GIT_ARCHIVE) {
    $env:OPENTOPIA_AGENT_TOOLS_GIT_ARCHIVE
  } else {
    $null
  }
  if ($ripgrepArchive) { $prepareArgs += @("-RipgrepArchive", $ripgrepArchive) }
  if ($gitArchive) { $prepareArgs += @("-GitArchive", $gitArchive) }
  & (Join-Path $PSScriptRoot "prepare-agent-tools-runtime.ps1") @prepareArgs
  return (Resolve-Path -LiteralPath $prepared).Path
}

function Assert-AgentToolsRuntimeSource {
  param([Parameter(Mandatory = $true)][string]$Path)

  $manifestPath = Join-Path $Path "agent-tools-runtime.json"
  if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) {
    throw "Agent tools runtime manifest not found: $manifestPath"
  }
  $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
  if ($manifest.schemaVersion -ne 1 -or $manifest.id -ne "ai.opentopia.agent-tools-runtime") {
    throw "Agent tools runtime manifest has an unsupported identity: $manifestPath"
  }
  if ($manifest.target -ne (Get-AgentToolsTargetId)) {
    throw "Agent tools runtime target $($manifest.target) does not match this build host"
  }

  $runtimeRoot = [System.IO.Path]::GetFullPath($Path)
  $runtimePrefix = $runtimeRoot + [System.IO.Path]::DirectorySeparatorChar
  foreach ($name in @("rg", "git")) {
    $tool = $manifest.tools.$name
    if (-not $tool.executable -or -not $tool.sha256 -or -not $tool.version) {
      throw "Agent tools runtime manifest has no valid $name artifact"
    }
    $executable = [System.IO.Path]::GetFullPath((Join-Path $runtimeRoot $tool.executable))
    if (-not $executable.StartsWith($runtimePrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Agent tools runtime $name path escapes its runtime root"
    }
    if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
      throw "Agent tools runtime $name executable not found: $executable"
    }
    $actualHash = (Get-FileHash -LiteralPath $executable -Algorithm SHA256).Hash
    if (-not $actualHash.Equals($tool.sha256, [System.StringComparison]::OrdinalIgnoreCase)) {
      throw "Agent tools runtime $name hash does not match its manifest"
    }
    $rawVersionOutput = & $executable --version
    $versionExitCode = $LASTEXITCODE
    $versionOutput = ($rawVersionOutput | Select-Object -First 1 | Out-String).Trim()
    if ($versionExitCode -ne 0 -or -not $versionOutput.Contains($tool.version)) {
      throw "Agent tools runtime $name version probe failed with exit code $versionExitCode; expected $($tool.version), got '$versionOutput'"
    }
  }
}

Push-Location $repoRoot
try {
  & (Join-Path $PSScriptRoot "test-provider-tool-cache-release.ps1")
  if ($LASTEXITCODE -ne 0) {
    throw "provider tool/cache release gate failed with exit code $LASTEXITCODE"
  }

  Write-Host "Building Rust server and Windows sandbox: cargo build --release -p opentopia-server -p opentopia-windows-sandbox"
  cargo build --release -p opentopia-server -p opentopia-windows-sandbox
  if ($LASTEXITCODE -ne 0) {
    throw "cargo build failed with exit code $LASTEXITCODE"
  }

  if (-not (Test-Path -LiteralPath $releaseServerBinary)) {
    throw "opentopia-server release binary not found at $releaseServerBinary"
  }

  Assert-RuntimeStagePath -Path $runtimeTempDir
  Assert-RuntimeStagePath -Path $runtimeStageDir
  if (Test-Path -LiteralPath $runtimeTempDir) {
    Remove-Item -LiteralPath $runtimeTempDir -Recurse -Force
  }
  New-Item -ItemType Directory -Force -Path $runtimeTempDir | Out-Null
  Copy-Item -LiteralPath $releaseServerBinary -Destination $stagedServerBinary -Force

  if (-not (Test-Path -LiteralPath $stagedServerBinary)) {
    throw "Failed to stage server binary at $stagedServerBinary"
  }

  Write-Host "Staged server binary for the runtime bundle: $stagedServerBinary"

  $resolvedOfficeRuntime = Resolve-OfficeRuntimeSource
  Assert-OfficeRuntimeSource -Path $resolvedOfficeRuntime
  Copy-Item -LiteralPath $resolvedOfficeRuntime -Destination $officeRuntimeStageDir -Recurse -Force
  Write-Host "Staged managed Office runtime: $officeRuntimeStageDir"

  $resolvedAgentToolsRuntime = Resolve-AgentToolsRuntimeSource
  if ($resolvedAgentToolsRuntime) {
    Assert-AgentToolsRuntimeSource -Path $resolvedAgentToolsRuntime
    Copy-Item -LiteralPath $resolvedAgentToolsRuntime -Destination $agentToolsRuntimeStageDir -Recurse -Force
    Assert-AgentToolsRuntimeSource -Path $agentToolsRuntimeStageDir
    Write-Host "Staged managed agent tools runtime: $agentToolsRuntimeStageDir"
  }

  $sandboxProtocol = $null
  if ($isWindowsHost) {
    if (-not (Test-Path -LiteralPath $releaseSandboxBinary)) {
      throw "opentopia-sandbox release binary not found at $releaseSandboxBinary"
    }
    $protocolJson = (& $releaseSandboxBinary protocol --json | Out-String).Trim()
    if ($LASTEXITCODE -ne 0) {
      throw "opentopia-sandbox protocol handshake failed with exit code $LASTEXITCODE"
    }
    $sandboxProtocol = $protocolJson | ConvertFrom-Json
    if ($sandboxProtocol.schema -ne "ai.opentopia.sandbox.protocol" -or $sandboxProtocol.protocolVersion -lt 1) {
      throw "opentopia-sandbox returned an invalid protocol descriptor: $protocolJson"
    }
    New-Item -ItemType Directory -Force -Path $sandboxStageDir | Out-Null
    Copy-Item -LiteralPath $releaseSandboxBinary -Destination $stagedSandboxBinary -Force
    if (-not (Test-Path -LiteralPath $stagedSandboxBinary)) {
      throw "Failed to stage OpenTopia Windows sandbox at $stagedSandboxBinary"
    }
    Write-Host "Staged first-party Windows sandbox for the runtime bundle: $stagedSandboxBinary"
  }
  $artifacts = [ordered]@{
    server = [ordered]@{
      path = $serverBinaryName
      sha256 = (Get-FileHash -LiteralPath $stagedServerBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    }
  }
  $artifacts.officeRuntime = [ordered]@{
    path = "office-runtime/office-runtime.json"
    sha256 = (Get-FileHash -LiteralPath (Join-Path $officeRuntimeStageDir "office-runtime.json") -Algorithm SHA256).Hash.ToLowerInvariant()
  }
  if ($resolvedAgentToolsRuntime) {
    $artifacts.agentTools = [ordered]@{
      path = "agent-tools/agent-tools-runtime.json"
      sha256 = (Get-FileHash -LiteralPath (Join-Path $agentToolsRuntimeStageDir "agent-tools-runtime.json") -Algorithm SHA256).Hash.ToLowerInvariant()
    }
  }
  if ($isWindowsHost) {
    $artifacts.sandbox = [ordered]@{
      path = "opentopia-sandbox/$sandboxBinaryName"
      sha256 = (Get-FileHash -LiteralPath $stagedSandboxBinary -Algorithm SHA256).Hash.ToLowerInvariant()
    }
  }
  $manifest = [ordered]@{
    schemaVersion = 1
    createdAt = [DateTime]::UtcNow.ToString("o")
    sandboxProtocol = $sandboxProtocol
    artifacts = $artifacts
  }
  $manifestJson = $manifest | ConvertTo-Json -Depth 8
  [System.IO.File]::WriteAllText(
    $runtimeManifestPath,
    $manifestJson,
    [System.Text.UTF8Encoding]::new($false)
  )

  $runtimeBackupDir = "$runtimeStageDir.previous-$PID"
  Assert-RuntimeStagePath -Path $runtimeBackupDir
  if (Test-Path -LiteralPath $runtimeBackupDir) {
    Remove-Item -LiteralPath $runtimeBackupDir -Recurse -Force
  }
  if (Test-Path -LiteralPath $runtimeStageDir) {
    Move-Item -LiteralPath $runtimeStageDir -Destination $runtimeBackupDir
  }
  try {
    Move-Item -LiteralPath $runtimeTempDir -Destination $runtimeStageDir
  } catch {
    if (Test-Path -LiteralPath $runtimeBackupDir) {
      Move-Item -LiteralPath $runtimeBackupDir -Destination $runtimeStageDir
    }
    throw
  }
  if (Test-Path -LiteralPath $runtimeBackupDir) {
    Remove-Item -LiteralPath $runtimeBackupDir -Recurse -Force
  }
  Write-Host "Published verified runtime bundle: $runtimeStageDir"

  if ($StageOnly) {
    Write-Host "Runtime bundle staging completed; Electron packaging was skipped."
    return
  }

  Invoke-Pnpm --filter @opentopia/desktop build
  $electronBuilderArgs = @()
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

  $electronBuilderName = if ($isWindowsHost) { "electron-builder.CMD" } else { "electron-builder" }
  $localElectronBuilder = Join-Path $desktopRoot "node_modules\.bin\$electronBuilderName"
  if (Test-Path -LiteralPath $localElectronBuilder) {
    Push-Location $desktopRoot
    try {
      & $localElectronBuilder @electronBuilderArgs
      if ($LASTEXITCODE -ne 0) {
        throw "electron-builder failed with exit code $LASTEXITCODE"
      }
    } finally {
      Pop-Location
    }
  } else {
    Invoke-Pnpm --filter @opentopia/desktop exec electron-builder @electronBuilderArgs
  }
} finally {
  if (Test-Path -LiteralPath $runtimeTempDir) {
    Assert-RuntimeStagePath -Path $runtimeTempDir
    Remove-Item -LiteralPath $runtimeTempDir -Recurse -Force
  }
  Pop-Location
}
