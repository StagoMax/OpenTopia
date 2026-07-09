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
}
