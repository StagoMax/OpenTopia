$ErrorActionPreference = "Stop"

function Add-PathEntry {
  param([Parameter(Mandatory = $true)][string]$PathEntry)

  if ([string]::IsNullOrWhiteSpace($PathEntry) -or -not (Test-Path $PathEntry)) {
    return
  }

  $entries = $env:PATH -split ';'
  if ($entries -notcontains $PathEntry) {
    $env:PATH = "$PathEntry;$env:PATH"
  }
}

function Resolve-MingwBin {
  if ($env:OPENTOPIA_MINGW_BIN -and (Test-Path $env:OPENTOPIA_MINGW_BIN)) {
    return $env:OPENTOPIA_MINGW_BIN
  }

  $candidates = @(
    (Join-Path $env:LOCALAPPDATA "Microsoft\WinGet\Packages\BrechtSanders.WinLibs.POSIX.UCRT_Microsoft.Winget.Source_8wekyb3d8bbwe\mingw64\bin"),
    "C:\msys64\ucrt64\bin",
    "C:\msys64\mingw64\bin"
  )

  foreach ($candidate in $candidates) {
    if (Test-Path (Join-Path $candidate "gcc.exe")) {
      return $candidate
    }
  }

  $gcc = Get-Command gcc -ErrorAction SilentlyContinue
  if ($gcc) {
    return Split-Path -Parent $gcc.Source
  }

  return $null
}

function Resolve-OpenTopiaEnvFile {
  if ($env:OPENTOPIA_ENV_FILE -and (Test-Path $env:OPENTOPIA_ENV_FILE)) {
    return $env:OPENTOPIA_ENV_FILE
  }

  $repoRoot = Split-Path -Parent $PSScriptRoot
  $workspaceRoot = Split-Path -Parent $repoRoot
  $creditReviewProjectName = -join ([char[]](0x4FE1, 0x8D37, 0x5BA1, 0x6838, 0x52A9, 0x624B))
  $candidates = @(
    (Join-Path $repoRoot ".env"),
    (Join-Path (Join-Path $workspaceRoot $creditReviewProjectName) ".env")
  )

  foreach ($candidate in $candidates) {
    if (Test-Path $candidate) {
      return $candidate
    }
  }

  if (Test-Path $workspaceRoot) {
    foreach ($directory in Get-ChildItem -LiteralPath $workspaceRoot -Directory -ErrorAction SilentlyContinue) {
      $candidate = Join-Path $directory.FullName ".env"
      if (-not (Test-Path $candidate)) {
        continue
      }

      $match = Select-String `
        -LiteralPath $candidate `
        -SimpleMatch `
        -Pattern "CREDIT_REVIEW_LLM_API_KEY", "AUDIT_COPILOT_LLM_API_KEY" `
        -Quiet
      if ($match) {
        return $candidate
      }
    }
  }

  return $null
}

function ConvertFrom-DotEnvValue {
  param([Parameter(Mandatory = $true)][string]$Value)

  $trimmed = $Value.Trim()
  if ($trimmed.Length -ge 2) {
    $first = $trimmed.Substring(0, 1)
    $last = $trimmed.Substring($trimmed.Length - 1, 1)
    if (($first -eq '"' -and $last -eq '"') -or ($first -eq "'" -and $last -eq "'")) {
      return $trimmed.Substring(1, $trimmed.Length - 2)
    }
  }

  return $trimmed
}

function Import-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if ([string]::IsNullOrWhiteSpace($line) -or $line.StartsWith("#")) {
      return
    }
    if ($line.StartsWith("export ")) {
      $line = $line.Substring(7).Trim()
    }

    $separator = $line.IndexOf("=")
    if ($separator -le 0) {
      return
    }

    $key = $line.Substring(0, $separator).Trim()
    $value = ConvertFrom-DotEnvValue $line.Substring($separator + 1)
    if ($key -and -not [Environment]::GetEnvironmentVariable($key, "Process")) {
      [Environment]::SetEnvironmentVariable($key, $value, "Process")
    }
  }
}

function Set-EnvFromAliases {
  param(
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][string[]]$Aliases
  )

  if ([Environment]::GetEnvironmentVariable($Target, "Process")) {
    return
  }

  foreach ($alias in $Aliases) {
    $value = [Environment]::GetEnvironmentVariable($alias, "Process")
    if ($value) {
      [Environment]::SetEnvironmentVariable($Target, $value, "Process")
      return
    }
  }
}

$opentopiaEnvFile = Resolve-OpenTopiaEnvFile
if ($opentopiaEnvFile) {
  $env:OPENTOPIA_ENV_FILE = $opentopiaEnvFile
  Import-DotEnvFile $opentopiaEnvFile
}

Set-EnvFromAliases "OPENTOPIA_API_KEY" @(
  "AUDIT_COPILOT_LLM_API_KEY",
  "CREDIT_REVIEW_LLM_API_KEY",
  "OPENAI_API_KEY"
)
Set-EnvFromAliases "OPENTOPIA_OPENAI_BASE_URL" @(
  "AUDIT_COPILOT_LLM_BASE_URL",
  "CREDIT_REVIEW_LLM_BASE_URL",
  "OPENAI_BASE_URL"
)
Set-EnvFromAliases "OPENTOPIA_MODEL" @(
  "AUDIT_COPILOT_LLM_MODEL",
  "CREDIT_REVIEW_LLM_MODEL",
  "CREDIT_REVIEW_LLM_CHEAP_MODEL",
  "CREDIT_REVIEW_LLM_STRONG_MODEL"
)

if (-not $env:OPENTOPIA_API_TOKEN) {
  # Local verification only. Electron replaces this with a fresh random token per launch.
  $env:OPENTOPIA_API_TOKEN = "opentopia-local-verification-token-0123456789abcdef0123456789abcdef"
}

if ($env:OS -eq "Windows_NT") {
  Add-PathEntry (Join-Path $env:USERPROFILE ".cargo\bin")

  $toolchain = if ($env:OPENTOPIA_RUST_TOOLCHAIN) {
    $env:OPENTOPIA_RUST_TOOLCHAIN
  } else {
    "stable-x86_64-pc-windows-gnu"
  }

  $env:RUSTUP_TOOLCHAIN = $toolchain

  $mingwBin = Resolve-MingwBin
  if ($mingwBin) {
    Add-PathEntry $mingwBin
  }

  if (-not (Get-Command rustup -ErrorAction SilentlyContinue)) {
    throw "rustup was not found. Install Rust with winget install Rustlang.Rustup, then rerun this script."
  }

  $installed = rustup toolchain list | Select-String -SimpleMatch $toolchain
  if (-not $installed) {
    rustup toolchain install $toolchain --profile minimal --component rustfmt --component clippy
  }

  if (-not (Get-Command gcc -ErrorAction SilentlyContinue)) {
    throw "gcc was not found. Install WinLibs with winget install BrechtSanders.WinLibs.POSIX.UCRT, or set OPENTOPIA_MINGW_BIN to mingw64\bin."
  }

  if (-not $env:OPENTOPIA_CODEX_SANDBOX_BIN) {
    $codexSandbox = Join-Path $env:USERPROFILE ".codex\plugins\.plugin-appserver\codex.exe"
    if (Test-Path -LiteralPath $codexSandbox) {
      $env:OPENTOPIA_CODEX_SANDBOX_BIN = $codexSandbox
    }
  }
}

if (-not $env:OPENTOPIA_SANDBOX_MODE) {
  $env:OPENTOPIA_SANDBOX_MODE = "best_effort"
}
if (-not $env:OPENTOPIA_SANDBOX_NETWORK) {
  $env:OPENTOPIA_SANDBOX_NETWORK = "deny"
}
