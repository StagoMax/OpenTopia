param()

$ErrorActionPreference = "Stop"
$repoRoot = (Resolve-Path (Join-Path $PSScriptRoot "..")).Path

Push-Location $repoRoot
try {
  Write-Host "Release gate: provider tool capability tiers and cache ordering"
  cargo test -p opentopia-core release_gate_ -- --nocapture
  if ($LASTEXITCODE -ne 0) {
    throw "provider tool/cache release gate failed with exit code $LASTEXITCODE"
  }
} finally {
  Pop-Location
}
