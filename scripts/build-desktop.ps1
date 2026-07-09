$ErrorActionPreference = "Stop"

. "$PSScriptRoot\dev-env.ps1"

Push-Location (Split-Path -Parent $PSScriptRoot)
try {
  cargo build --release -p opentopia-server
  New-Item -ItemType Directory -Force -Path apps\desktop\resources | Out-Null
  if (Test-Path target\release\opentopia-server.exe) {
    Copy-Item target\release\opentopia-server.exe apps\desktop\resources\opentopia-server.exe -Force
  } elseif (Test-Path target\release\opentopia-server) {
    Copy-Item target\release\opentopia-server apps\desktop\resources\opentopia-server -Force
  } else {
    throw "opentopia-server release binary not found"
  }
  pnpm.cmd --filter @opentopia/desktop dist
} finally {
  Pop-Location
}
