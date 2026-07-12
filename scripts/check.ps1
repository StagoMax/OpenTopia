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

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  cargo check --workspace
  if ($LASTEXITCODE -ne 0) {
    throw "cargo check failed with exit code $LASTEXITCODE"
  }
  Invoke-Pnpm --filter @opentopia/desktop typecheck
  Invoke-Pnpm --filter @opentopia/desktop build
} finally {
  Pop-Location
}
