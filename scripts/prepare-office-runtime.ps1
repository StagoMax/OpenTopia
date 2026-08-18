param(
  [Parameter(Mandatory = $true)][string]$Python,
  [Parameter(Mandatory = $true)][string]$Output
)

$ErrorActionPreference = "Stop"

$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path
$requirements = Join-Path $repoRoot "runtime\office\requirements.lock"
$pythonPath = (Resolve-Path -LiteralPath $Python).Path
$outputPath = [System.IO.Path]::GetFullPath($Output)
$temporaryPath = "$outputPath.tmp-$PID"

function Assert-ManagedRuntimePath {
  param([Parameter(Mandatory = $true)][string]$Path)
  $fullPath = [System.IO.Path]::GetFullPath($Path)
  $repoPrefix = $repoRoot + [System.IO.Path]::DirectorySeparatorChar
  if (-not $fullPath.StartsWith($repoPrefix, [System.StringComparison]::OrdinalIgnoreCase)) {
    throw "Office runtime output must stay inside this repository: $fullPath"
  }
}

function Get-RelativeRuntimePath {
  param([Parameter(Mandatory = $true)][string]$Root, [Parameter(Mandatory = $true)][string]$Path)
  $rootUri = [System.Uri]::new($Root.TrimEnd([System.IO.Path]::DirectorySeparatorChar) + [System.IO.Path]::DirectorySeparatorChar)
  $pathUri = [System.Uri]::new($Path)
  return [System.Uri]::UnescapeDataString($rootUri.MakeRelativeUri($pathUri).ToString()).Replace("/", "/")
}

Assert-ManagedRuntimePath -Path $outputPath
Assert-ManagedRuntimePath -Path $temporaryPath
if (Test-Path -LiteralPath $temporaryPath) {
  Remove-Item -LiteralPath $temporaryPath -Recurse -Force
}
if (Test-Path -LiteralPath $outputPath) {
  throw "Office runtime output already exists: $outputPath. Remove or choose a fresh output directory."
}

try {
  New-Item -ItemType Directory -Force -Path $temporaryPath | Out-Null
  $venvPath = Join-Path $temporaryPath "python"
  & $pythonPath -m venv $venvPath
  if ($LASTEXITCODE -ne 0) { throw "Python virtual-environment creation failed with exit code $LASTEXITCODE" }

  $managedPython = if ($IsWindows -or $env:OS -eq "Windows_NT") {
    Join-Path $venvPath "Scripts\python.exe"
  } else {
    Join-Path $venvPath "bin/python"
  }
  if (-not (Test-Path -LiteralPath $managedPython)) {
    throw "Managed Python was not created at $managedPython"
  }

  & $managedPython -m pip install --disable-pip-version-check --no-input -r $requirements
  if ($LASTEXITCODE -ne 0) { throw "Installing Office runtime requirements failed with exit code $LASTEXITCODE" }
  $openpyxlVersion = (& $managedPython -I -c "import importlib.metadata; import openpyxl; print(importlib.metadata.version('openpyxl'))" | Out-String).Trim()
  if ($LASTEXITCODE -ne 0 -or $openpyxlVersion -ne "3.1.5") {
    throw "Managed Python cannot import the pinned openpyxl==3.1.5 package"
  }

  $manifest = [ordered]@{
    schemaVersion = 1
    id = "ai.opentopia.office-runtime"
    version = "1.0.0"
    python = [ordered]@{
      path = Get-RelativeRuntimePath -Root $temporaryPath -Path $managedPython
      sha256 = (Get-FileHash -LiteralPath $managedPython -Algorithm SHA256).Hash.ToLowerInvariant()
    }
    packages = [ordered]@{ openpyxl = $openpyxlVersion }
  }
  [System.IO.File]::WriteAllText(
    (Join-Path $temporaryPath "office-runtime.json"),
    ($manifest | ConvertTo-Json -Depth 5),
    [System.Text.UTF8Encoding]::new($false)
  )
  Move-Item -LiteralPath $temporaryPath -Destination $outputPath
  Write-Host "Prepared OpenTopia Office runtime: $outputPath"
} catch {
  if (Test-Path -LiteralPath $temporaryPath) {
    Remove-Item -LiteralPath $temporaryPath -Recurse -Force
  }
  throw
}
