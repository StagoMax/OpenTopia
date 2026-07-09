$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  cargo check --workspace
  pnpm.cmd --filter @opentopia/desktop typecheck
  pnpm.cmd --filter @opentopia/desktop build
} finally {
  Pop-Location
}
