param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [Parameter(Mandatory = $true)][string]$UserDataDir,
  [string]$Profile = "AUDIT_COPILOT_LLM"
)

$ErrorActionPreference = "Stop"
$repoRoot = Split-Path -Parent $PSScriptRoot
$electron = @(
  (Join-Path $repoRoot "apps\desktop\node_modules\.bin\electron.CMD"),
  (Join-Path $repoRoot "node_modules\.bin\electron.CMD")
) | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1
if (-not $electron) {
  throw "Electron is not installed; run pnpm install first"
}

$runtimeUserData = Join-Path $repoRoot ".opentopia\safe-storage-runtime-$PID"
New-Item -ItemType Directory -Path $runtimeUserData -Force | Out-Null
try {
  & $electron `
    (Join-Path $PSScriptRoot "configure-provider-safe-storage.cjs") `
    "--env-file" $EnvFile `
    "--profile" $Profile `
    "--target-user-data" $UserDataDir `
    "--runtime-user-data" $runtimeUserData
  if ($LASTEXITCODE -ne 0) {
    throw "safeStorage configuration failed"
  }
} finally {
  if (Test-Path -LiteralPath $runtimeUserData) {
    $resolvedRoot = [IO.Path]::GetFullPath((Join-Path $repoRoot ".opentopia"))
    $resolvedTarget = [IO.Path]::GetFullPath($runtimeUserData)
    if (-not $resolvedTarget.StartsWith($resolvedRoot, [StringComparison]::OrdinalIgnoreCase)) {
      throw "Refusing to remove a runtime directory outside .opentopia"
    }
    Remove-Item -LiteralPath $resolvedTarget -Recurse -Force
  }
}
