param(
  [Parameter(Mandatory = $true)][string]$Output,
  [string]$PythonArchive,
  [string]$ArtifactCache
)

$ErrorActionPreference = "Stop"

. "$PSScriptRoot\runtime-download.ps1"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$lockPath = Join-Path $repoRoot "runtime\office\runtime-lock.json"
$lock = Get-Content -LiteralPath $lockPath -Raw | ConvertFrom-Json
$outputPath = [System.IO.Path]::GetFullPath($Output)
$temporaryPath = "$outputPath.tmp-$PID-$([Guid]::NewGuid().ToString('N'))"
$artifactCachePath = if ($ArtifactCache) {
  [System.IO.Path]::GetFullPath($ArtifactCache)
} elseif ($env:OPENTOPIA_OFFICE_RUNTIME_ARTIFACT_CACHE) {
  [System.IO.Path]::GetFullPath($env:OPENTOPIA_OFFICE_RUNTIME_ARTIFACT_CACHE)
} else {
  Join-Path $repoRoot "runtime\office\cache\downloads"
}

function Assert-RepositoryPath {
  param([Parameter(Mandatory = $true)][string]$Path, [Parameter(Mandatory = $true)][string]$Label)
  $fullPath = [System.IO.Path]::GetFullPath($Path)
  $repoPrefix = $repoRoot + [System.IO.Path]::DirectorySeparatorChar
  if (-not $fullPath.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "$Label must stay inside this repository: $fullPath"
  }
}

function Get-RelativeRuntimePath {
  param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
  $rootUri = [System.Uri]::new($Root.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar)
  $pathUri = [System.Uri]::new($Path)
  return [System.Uri]::UnescapeDataString($rootUri.MakeRelativeUri($pathUri).ToString()).Replace("\", "/")
}

function Get-CurrentTargetId {
  $isWindowsHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::Windows
  )
  $isMacHost = [System.Runtime.InteropServices.RuntimeInformation]::IsOSPlatform(
    [System.Runtime.InteropServices.OSPlatform]::OSX
  )
  $os = if ($isWindowsHost) { "windows" } elseif ($isMacHost) { "macos" } else { "linux" }
  $architecture = [System.Runtime.InteropServices.RuntimeInformation]::ProcessArchitecture.ToString().ToLowerInvariant()
  $arch = switch ($architecture) {
    "x64" { "x86_64" }
    "arm64" { "aarch64" }
    default { throw "Managed Office Python is unsupported on architecture $architecture" }
  }
  return "$os-$arch"
}

function Invoke-PythonProbe {
  param([Parameter(Mandatory = $true)][string]$Python)
  $script = "import importlib.metadata,json,sys; print(json.dumps({'python': '.'.join(map(str, sys.version_info[:3])), 'openpyxl': importlib.metadata.version('openpyxl'), 'et_xmlfile': importlib.metadata.version('et_xmlfile')}))"
  $result = (& $Python -I -c $script | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or -not $result) {
    throw "Managed Python probe failed at $Python"
  }
  return $result | ConvertFrom-Json
}

function Test-PreparedRuntime {
  param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$TargetId)
  try {
    $manifestPath = Join-Path $Root "office-runtime.json"
    if (-not (Test-Path -LiteralPath $manifestPath -PathType Leaf)) { return $false }
    $manifest = Get-Content -LiteralPath $manifestPath -Raw | ConvertFrom-Json
    if ($manifest.schemaVersion -ne 2 -or
        $manifest.id -ne "ai.opentopia.office-runtime" -or
        $manifest.version -ne $lock.runtimeVersion -or
        $manifest.target -ne $TargetId) { return $false }
    $python = Join-Path $Root $manifest.python.path
    if (-not (Test-RuntimeArtifactSha256 -Path $python -Expected $manifest.python.sha256)) { return $false }
    $probe = Invoke-PythonProbe -Python $python
    return $probe.python -eq $lock.pythonVersion -and
      $probe.openpyxl -eq (($lock.packages | Where-Object name -eq "openpyxl").version) -and
      $probe.et_xmlfile -eq (($lock.packages | Where-Object name -eq "et_xmlfile").version)
  } catch {
    return $false
  }
}

if ($lock.schemaVersion -ne 1 -or $lock.id -ne "ai.opentopia.office-runtime.lock") {
  throw "Unsupported Office runtime lock: $lockPath"
}
$targetId = Get-CurrentTargetId
$pythonAsset = $lock.targets.$targetId
if (-not $pythonAsset) { throw "Office runtime lock has no artifact for $targetId" }

Assert-RepositoryPath -Path $outputPath -Label "Office runtime output"
Assert-RepositoryPath -Path $temporaryPath -Label "Office runtime temporary path"
Assert-RepositoryPath -Path $artifactCachePath -Label "Office runtime artifact cache"
if (Test-PreparedRuntime -Root $outputPath -TargetId $targetId) {
  Write-Host "Reusing prepared OpenTopia Office runtime: $outputPath"
  return
}
if (Test-Path -LiteralPath $outputPath) {
  throw "Existing Office runtime is invalid or stale: $outputPath. Remove it before rebuilding."
}
if (Test-Path -LiteralPath $temporaryPath) {
  Remove-Item -LiteralPath $temporaryPath -Recurse -Force
}

try {
  New-Item -ItemType Directory -Force -Path $temporaryPath | Out-Null
  $archivePath = if ($PythonArchive) {
    $resolved = (Resolve-Path -LiteralPath $PythonArchive).Path
    if (-not (Test-RuntimeArtifactSha256 -Path $resolved -Expected $pythonAsset.sha256)) {
      throw "Offline standalone Python archive failed SHA-256 verification: $resolved"
    }
    $resolved
  } else {
    Invoke-VerifiedRuntimeDownload `
      -Uri $pythonAsset.url `
      -Destination (Join-Path $artifactCachePath $pythonAsset.fileName) `
      -Sha256 $pythonAsset.sha256 `
      -MaxBytes $pythonAsset.maxBytes
  }

  & tar -xzf $archivePath -C $temporaryPath
  if ($LASTEXITCODE -ne 0) { throw "Extracting standalone Python failed with exit code $LASTEXITCODE" }
  $managedPython = Join-Path $temporaryPath $pythonAsset.pythonPath
  if (-not (Test-Path -LiteralPath $managedPython -PathType Leaf)) {
    throw "Standalone Python executable was not created at $managedPython"
  }

  $wheelPaths = @()
  foreach ($package in $lock.packages) {
    $wheelPaths += Invoke-VerifiedRuntimeDownload `
      -Uri $package.url `
      -Destination (Join-Path $artifactCachePath $package.fileName) `
      -Sha256 $package.sha256 `
      -MaxBytes $package.maxBytes
  }
  & $managedPython -I -m pip install `
    --disable-pip-version-check --no-input --no-index --no-deps --no-compile `
    @wheelPaths
  if ($LASTEXITCODE -ne 0) {
    throw "Installing verified Office runtime wheels failed with exit code $LASTEXITCODE"
  }

  $probe = Invoke-PythonProbe -Python $managedPython
  $openpyxlVersion = ($lock.packages | Where-Object name -eq "openpyxl").version
  $etXmlfileVersion = ($lock.packages | Where-Object name -eq "et_xmlfile").version
  if ($probe.python -ne $lock.pythonVersion -or
      $probe.openpyxl -ne $openpyxlVersion -or
      $probe.et_xmlfile -ne $etXmlfileVersion) {
    throw "Prepared Office Python does not match the pinned runtime lock"
  }

  $manifest = [ordered]@{
    schemaVersion = 2
    id = "ai.opentopia.office-runtime"
    version = $lock.runtimeVersion
    target = $targetId
    python = [ordered]@{
      path = Get-RelativeRuntimePath -Root $temporaryPath -Path $managedPython
      sha256 = (Get-FileHash -LiteralPath $managedPython -Algorithm SHA256).Hash.ToLowerInvariant()
      version = $lock.pythonVersion
      distribution = [ordered]@{
        provider = "astral-sh/python-build-standalone"
        release = $lock.pythonRelease
        targetTriple = $pythonAsset.targetTriple
        archiveSha256 = $pythonAsset.sha256
      }
    }
    packages = [ordered]@{
      openpyxl = $openpyxlVersion
      etXmlfile = $etXmlfileVersion
    }
  }
  [System.IO.File]::WriteAllText(
    (Join-Path $temporaryPath "office-runtime.json"),
    ($manifest | ConvertTo-Json -Depth 8),
    [System.Text.UTF8Encoding]::new($false)
  )

  try {
    Move-Item -LiteralPath $temporaryPath -Destination $outputPath
  } catch {
    if (-not (Test-PreparedRuntime -Root $outputPath -TargetId $targetId)) { throw }
    Remove-Item -LiteralPath $temporaryPath -Recurse -Force -ErrorAction SilentlyContinue
  }
  Write-Host "Prepared OpenTopia Office runtime: $outputPath"
} catch {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Recurse -Force
  }
  throw
}
