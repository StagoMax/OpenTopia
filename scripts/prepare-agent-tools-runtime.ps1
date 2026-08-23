param(
  [string]$Output,
  [string]$RipgrepArchive,
  [string]$GitArchive,
  [string]$ArtifactCache
)

$ErrorActionPreference = "Stop"

. "$PSScriptRoot\runtime-download.ps1"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$lockPath = Join-Path $repoRoot "runtime\agent-tools\runtime-lock.json"
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
$artifactCachePath = if ($ArtifactCache) {
  [System.IO.Path]::GetFullPath($ArtifactCache)
} elseif ($env:OPENTOPIA_AGENT_TOOLS_ARTIFACT_CACHE) {
  [System.IO.Path]::GetFullPath($env:OPENTOPIA_AGENT_TOOLS_ARTIFACT_CACHE)
} else {
  Join-Path $repoRoot "runtime\agent-tools\cache\downloads"
}

function Assert-RepositoryPath {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Label
  )

  $fullPath = [System.IO.Path]::GetFullPath($Path)
  $repoPrefix = $repoRoot + [System.IO.Path]::DirectorySeparatorChar
  if (-not $fullPath.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "$Label must stay inside this repository: $fullPath"
  }
}

function Get-RelativeRuntimePath {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string]$Path
  )

  $rootUri = [System.Uri]::new(
    $Root.TrimEnd([System.IO.Path]::DirectorySeparatorChar) +
      [System.IO.Path]::DirectorySeparatorChar
  )
  $pathUri = [System.Uri]::new($Path)
  return [System.Uri]::UnescapeDataString(
    $rootUri.MakeRelativeUri($pathUri).ToString()
  ).Replace("\", "/")
}

function Get-CurrentAgentToolsTargetId {
  if (-not [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
      [System.Runtime.InteropServices.OSPlatform]::Windows
    )) {
    throw "The core agent tools bundle is currently prepared only for Windows"
  }

  $architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
  $arch = switch ($architecture) {
    "x64" { "x86_64" }
    "arm64" { "aarch64" }
    default { throw "The agent tools runtime is unsupported on architecture $architecture" }
  }
  return "windows-$arch"
}

function Resolve-VerifiedArchive {
  param(
    [string]$OfflineArchive,
    [Parameter(Mandatory = $true)]$Descriptor,
    [Parameter(Mandatory = $true)][string]$Label
  )

  if ($OfflineArchive) {
    $resolved = (Resolve-Path -LiteralPath $OfflineArchive).Path
    if (-not (Test-RuntimeArtifactSha256 -Path $resolved -Expected $Descriptor.sha256)) {
      throw "$Label archive failed SHA-256 verification: $resolved"
    }
    return $resolved
  }

  return Invoke-VerifiedRuntimeDownload `
    -Uri $Descriptor.url `
    -Destination (Join-Path $artifactCachePath $Descriptor.fileName) `
    -Sha256 $Descriptor.sha256 `
    -MaxBytes $Descriptor.maxBytes `
    -UserAgent "OpenTopia agent tools runtime preparer"
}

function Invoke-AgentToolProbe {
  param(
    [Parameter(Mandatory = $true)][string]$Executable,
    [Parameter(Mandatory = $true)][string]$ExpectedVersion,
    [Parameter(Mandatory = $true)][string]$Label
  )

  $rawOutput = & $Executable --version
  $exitCode = $LASTEXITCODE
  $result = ($rawOutput | Select-Object -First 1 | Out-String).Trim()
  if ($exitCode -ne 0 -or -not $result.Contains($ExpectedVersion)) {
    throw "$Label probe failed at $Executable with exit code $exitCode; expected version $ExpectedVersion, got '$result'"
  }
  return $result
}

function Test-PreparedAgentToolsRuntime {
  param(
    [Parameter(Mandatory = $true)][string]$Root,
    [Parameter(Mandatory = $true)][string]$TargetId
  )

  try {
    $manifestPath = Join-Path $Root "agent-tools-runtime.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { return $false }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.schemaVersion -ne 1 -or
        $manifest.id -ne "ai.opentopia.agent-tools-runtime" -or
        $manifest.version -ne $lock.runtimeVersion -or
        $manifest.target -ne $TargetId) {
      return $false
    }

    foreach ($name in @("rg", "git")) {
      $tool = $manifest.tools.$name
      $expected = $lock.tools.$name
      if (-not $tool -or $tool.version -ne $expected.version) { return $false }
      $executable = Join-Path $Root $tool.executable
      if (-not (Test-RuntimeArtifactSha256 -Path $executable -Expected $tool.sha256)) {
        return $false
      }
      Invoke-AgentToolProbe `
        -Executable $executable `
        -ExpectedVersion $expected.version `
        -Label $name | Out-Null
    }
    return $true
  } catch {
    return $false
  }
}

if ($lock.schemaVersion -ne 1 -or $lock.id -ne "ai.opentopia.agent-tools-runtime.lock") {
  throw "Unsupported agent tools runtime lock: $lockPath"
}

$targetId = Get-CurrentAgentToolsTargetId
$target = $lock.targets.$targetId
if (-not $target) { throw "Agent tools runtime lock has no artifacts for $targetId" }
$outputPath = if ($Output) {
  [System.IO.Path]::GetFullPath($Output)
} else {
  Join-Path $repoRoot "runtime\agent-tools\cache\$($lock.runtimeVersion)\$targetId"
}
$temporaryPath = "$outputPath.tmp-$PID-$([Guid]::NewGuid().ToString('N'))"

$resolvedRipgrepArchive = if ($RipgrepArchive) {
  $RipgrepArchive
} elseif ($env:OPENTOPIA_AGENT_TOOLS_RG_ARCHIVE) {
  $env:OPENTOPIA_AGENT_TOOLS_RG_ARCHIVE
} else {
  $null
}
$resolvedGitArchive = if ($GitArchive) {
  $GitArchive
} elseif ($env:OPENTOPIA_AGENT_TOOLS_GIT_ARCHIVE) {
  $env:OPENTOPIA_AGENT_TOOLS_GIT_ARCHIVE
} else {
  $null
}

Assert-RepositoryPath -Path $outputPath -Label "Agent tools runtime output"
Assert-RepositoryPath -Path $temporaryPath -Label "Agent tools runtime temporary path"
Assert-RepositoryPath -Path $artifactCachePath -Label "Agent tools artifact cache"

if (Test-PreparedAgentToolsRuntime -Root $outputPath -TargetId $targetId) {
  Write-Host "Reusing prepared OpenTopia agent tools runtime: $outputPath"
  return
}
if (Test-Path -LiteralPath $outputPath) {
  throw "Existing agent tools runtime is invalid or stale: $outputPath. Remove it before rebuilding."
}
if (Test-Path -LiteralPath $temporaryPath) {
  Remove-Item -LiteralPath $temporaryPath -Recurse -Force
}

try {
  $ripgrepArchivePath = Resolve-VerifiedArchive `
    -OfflineArchive $resolvedRipgrepArchive `
    -Descriptor $target.ripgrep `
    -Label "ripgrep"
  $gitArchivePath = Resolve-VerifiedArchive `
    -OfflineArchive $resolvedGitArchive `
    -Descriptor $target.git `
    -Label "MinGit"

  $ripgrepExtractPath = Join-Path $temporaryPath ".extract-ripgrep"
  $binPath = Join-Path $temporaryPath "bin"
  $gitRoot = Join-Path $temporaryPath "git"
  $ripgrepLicensePath = Join-Path $temporaryPath "licenses\ripgrep"
  New-Item -ItemType Directory -Force -Path $ripgrepExtractPath, $binPath, $gitRoot, $ripgrepLicensePath | Out-Null

  Expand-Archive -LiteralPath $ripgrepArchivePath -DestinationPath $ripgrepExtractPath
  $ripgrepSource = Join-Path $ripgrepExtractPath $target.ripgrep.executablePath
  if (-not (Test-Path -LiteralPath $ripgrepSource -PathType Leaf)) {
    throw "ripgrep executable was not extracted at $ripgrepSource"
  }
  $ripgrepExecutable = Join-Path $binPath "rg.exe"
  Copy-Item -LiteralPath $ripgrepSource -Destination $ripgrepExecutable -Force
  $ripgrepSourceRoot = Split-Path -Parent $ripgrepSource
  foreach ($licenseName in @("LICENSE-MIT", "UNLICENSE")) {
    $license = Join-Path $ripgrepSourceRoot $licenseName
    if (Test-Path -LiteralPath $license -PathType Leaf) {
      Copy-Item -LiteralPath $license -Destination $ripgrepLicensePath -Force
    }
  }
  Remove-Item -LiteralPath $ripgrepExtractPath -Recurse -Force

  Expand-Archive -LiteralPath $gitArchivePath -DestinationPath $gitRoot
  $gitExecutable = Join-Path $gitRoot $target.git.executablePath
  if (-not (Test-Path -LiteralPath $gitExecutable -PathType Leaf)) {
    throw "MinGit executable was not extracted at $gitExecutable"
  }

  Invoke-AgentToolProbe `
    -Executable $ripgrepExecutable `
    -ExpectedVersion $lock.tools.rg.version `
    -Label "ripgrep" | Out-Null
  Invoke-AgentToolProbe `
    -Executable $gitExecutable `
    -ExpectedVersion $lock.tools.git.version `
    -Label "MinGit" | Out-Null

  $manifest = [ordered]@{
    schemaVersion = 1
    id = "ai.opentopia.agent-tools-runtime"
    version = $lock.runtimeVersion
    target = $targetId
    capabilities = @("workspace-search", "source-control")
    pathEntries = @("bin", "git/cmd")
    tools = [ordered]@{
      rg = [ordered]@{
        version = $lock.tools.rg.version
        executable = Get-RelativeRuntimePath -Root $temporaryPath -Path $ripgrepExecutable
        sha256 = (Get-FileHash -LiteralPath $ripgrepExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        provider = $lock.tools.rg.provider
        archiveSha256 = $target.ripgrep.sha256
      }
      git = [ordered]@{
        version = $lock.tools.git.version
        executable = Get-RelativeRuntimePath -Root $temporaryPath -Path $gitExecutable
        sha256 = (Get-FileHash -LiteralPath $gitExecutable -Algorithm SHA256).Hash.ToLowerInvariant()
        provider = $lock.tools.git.provider
        archiveSha256 = $target.git.sha256
      }
    }
  }
  [System.IO.File]::WriteAllText(
    (Join-Path $temporaryPath "agent-tools-runtime.json"),
    ($manifest | ConvertTo-Json -Depth 8),
    [System.Text.UTF8Encoding]::new($false)
  )

  try {
    Move-Item -LiteralPath $temporaryPath -Destination $outputPath
  } catch {
    if (-not (Test-PreparedAgentToolsRuntime -Root $outputPath -TargetId $targetId)) {
      throw
    }
    Remove-Item -LiteralPath $temporaryPath -Recurse -Force -ErrorAction SilentlyContinue
  }

  if (-not (Test-PreparedAgentToolsRuntime -Root $outputPath -TargetId $targetId)) {
    throw "Prepared agent tools runtime failed final verification"
  }
  Write-Host "Prepared OpenTopia agent tools runtime: $outputPath"
} catch {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Recurse -Force
  }
  throw
}
