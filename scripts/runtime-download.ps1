$ErrorActionPreference = "Stop"

function Test-RuntimeArtifactSha256 {
  param(
    [Parameter(Mandatory = $true)][string]$Path,
    [Parameter(Mandatory = $true)][string]$Expected
  )

  if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { return $false }
  $actual = (Get-FileHash -LiteralPath $Path -Algorithm SHA256).Hash
  return $actual.Equals($Expected, [System.StringComparison]::OrdinalIgnoreCase)
}

function Get-RuntimeDownloadRetryAfterSeconds {
  param($ErrorRecord)

  try {
    $value = $ErrorRecord.Exception.Response.Headers["Retry-After"]
    $seconds = 0
    if ([int]::TryParse([string]$value, [ref]$seconds)) {
      return [Math]::Min($seconds, 8)
    }
  } catch {}
  return $null
}

function Test-RuntimeDownloadRetryableError {
  param($ErrorRecord)

  try {
    $status = [int]$ErrorRecord.Exception.Response.StatusCode
    return $status -in @(429, 502, 503, 504)
  } catch {
    # No HTTP response means connection establishment, DNS, TLS, or timeout.
    return $true
  }
}

function Invoke-VerifiedRuntimeDownload {
  param(
    [Parameter(Mandatory = $true)][string]$Uri,
    [Parameter(Mandatory = $true)][string]$Destination,
    [Parameter(Mandatory = $true)][string]$Sha256,
    [Parameter(Mandatory = $true)][long]$MaxBytes,
    [string]$UserAgent = "OpenTopia runtime preparer"
  )

  if (Test-RuntimeArtifactSha256 -Path $Destination -Expected $Sha256) {
    return $Destination
  }
  if (Test-Path -LiteralPath $Destination) {
    throw "Cached artifact failed SHA-256 verification: $Destination"
  }

  $parent = Split-Path -Parent $Destination
  New-Item -ItemType Directory -Force -Path $parent | Out-Null

  for ($attempt = 1; $attempt -le 3; $attempt++) {
    $temporary = "$Destination.download-$PID-$([Guid]::NewGuid().ToString('N'))"
    try {
      Invoke-WebRequest -Uri $Uri -OutFile $temporary -TimeoutSec 900 -MaximumRedirection 10 -Headers @{
        "User-Agent" = $UserAgent
      }
      $length = (Get-Item -LiteralPath $temporary).Length
      if ($length -gt $MaxBytes) {
        throw "Downloaded artifact exceeds the configured $MaxBytes byte limit"
      }
      if (-not (Test-RuntimeArtifactSha256 -Path $temporary -Expected $Sha256)) {
        throw "Downloaded artifact failed SHA-256 verification: $Uri"
      }
      try {
        Move-Item -LiteralPath $temporary -Destination $Destination
      } catch {
        if (-not (Test-RuntimeArtifactSha256 -Path $Destination -Expected $Sha256)) {
          throw
        }
        Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
      }
      return $Destination
    } catch {
      Remove-Item -LiteralPath $temporary -Force -ErrorAction SilentlyContinue
      $message = $_.Exception.Message
      $integrityFailure = $message -match "SHA-256|exceeds the configured"
      if ($integrityFailure -or
          $attempt -ge 3 -or
          -not (Test-RuntimeDownloadRetryableError -ErrorRecord $_)) {
        throw
      }
      $backoff = [Math]::Min([Math]::Pow(2, $attempt - 1), 8)
      $retryAfter = Get-RuntimeDownloadRetryAfterSeconds -ErrorRecord $_
      if ($null -ne $retryAfter) {
        $backoff = [Math]::Max($backoff, $retryAfter)
      }
      Write-Warning "Runtime download attempt $attempt failed; retrying in $backoff seconds: $message"
      Start-Sleep -Seconds $backoff
    }
  }
}
