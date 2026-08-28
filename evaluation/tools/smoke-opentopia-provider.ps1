param(
    [Parameter(Mandatory = $true)]
    [string]$ServerBinary,

    [Parameter(Mandatory = $true)]
    [string]$EnvFile,

    [Parameter(Mandatory = $true)]
    [string]$ReportPath,

    [int]$Port = 18789,

    [int]$TurnTimeoutSeconds = 120,

    # Exercise one provider -> tool -> provider round trip in an isolated
    # temporary workspace. This is diagnostic-only and never a benchmark run.
    [switch]$ExerciseToolLoop
)

$ErrorActionPreference = 'Stop'

function Get-EvalSetting {
    param([string]$Name)

    foreach ($line in Get-Content -LiteralPath $EnvFile) {
        if ($line -match '^\s*#' -or $line -notmatch '=') {
            continue
        }
        $parts = $line -split '=', 2
        if ($parts[0].Trim() -eq $Name) {
            return $parts[1]
        }
    }
    throw "Required evaluation setting is missing: $Name"
}

function New-InternalToken {
    $bytes = New-Object byte[] 32
    [System.Security.Cryptography.RandomNumberGenerator]::Fill($bytes)
    return [Convert]::ToBase64String($bytes)
}

if (-not (Test-Path -LiteralPath $ServerBinary)) {
    throw 'Server binary was not found.'
}
if (-not (Test-Path -LiteralPath $EnvFile)) {
    throw 'Evaluation env file was not found.'
}

$apiKey = Get-EvalSetting -Name 'AUDIT_COPILOT_LLM_API_KEY'
$baseUrl = Get-EvalSetting -Name 'AUDIT_COPILOT_LLM_BASE_URL'
$model = Get-EvalSetting -Name 'AUDIT_COPILOT_LLM_MODEL'
$token = New-InternalToken
$containerName = "opentopia-provider-smoke-$Port"
$serverScript = "mkdir -p /tmp/opentopia-smoke-workspace && exec /opentopia-server --host 0.0.0.0 --port $Port --db /tmp/opentopia-smoke.db --permission full-access"
$containerStarted = $false
$stage = 'start_container'

function Invoke-OpenTopiaApi {
    param(
        [string]$Method,
        [string]$Path,
        [object]$Body = $null
    )

    $headers = @{ Authorization = "Bearer $token" }
    $params = @{
        Method = $Method
        Uri = "http://127.0.0.1:$Port$Path"
        Headers = $headers
        TimeoutSec = 30
    }
    if ($null -ne $Body) {
        $params.ContentType = 'application/json'
        $params.Body = $Body | ConvertTo-Json -Depth 100 -Compress
    }
    Invoke-RestMethod @params
}

try {
    if (docker ps -a --filter "name=^/$containerName$" --format '{{.ID}}') {
        throw 'A smoke container with the requested name already exists.'
    }
    $mount = "$ServerBinary`:/opentopia-server:ro"
    $dockerArgs = @(
        'run', '-d', '--rm', '--name', $containerName,
        '-p', "127.0.0.1:$Port`:$Port",
        '-v', $mount,
        '-e', "OPENTOPIA_API_KEY=$apiKey",
        '-e', "OPENTOPIA_OPENAI_BASE_URL=$baseUrl",
        '-e', "OPENTOPIA_MODEL=$model",
        '-e', "OPENTOPIA_API_TOKEN=$token",
        '-e', 'OPENTOPIA_DB=/tmp/opentopia-smoke.db',
        '-e', 'OPENTOPIA_PERMISSION=full-access',
        '-e', 'OPENTOPIA_SANDBOX_MODE=danger-full-access',
        '-e', 'OPENTOPIA_SANDBOX_ENFORCEMENT=disabled',
        '-e', 'OPENTOPIA_SANDBOX_NETWORK=inherit',
        'alpine:3.20', 'sh', '-c', $serverScript
    )
    $null = & docker.exe @dockerArgs
    if ($LASTEXITCODE -ne 0) {
        throw 'Smoke container did not start.'
    }
    $containerStarted = $true

    $stage = 'wait_for_health'
    $health = $null
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while ([DateTime]::UtcNow -lt $deadline) {
        try {
            $health = Invoke-OpenTopiaApi -Method 'GET' -Path '/health'
            if ($health.ok -eq $true -and $health.service -eq 'opentopia-server') {
                break
            }
        } catch {
            Start-Sleep -Seconds 1
        }
    }
    if ($null -eq $health -or $health.ok -ne $true) {
        throw 'Smoke server did not become healthy.'
    }

    $stage = 'configure_provider'
    $settings = Invoke-OpenTopiaApi -Method 'GET' -Path '/api/settings'
    $providerId = $settings.activeProviderId
    $provider = @($settings.providers) | Where-Object { $_.id -eq $providerId } | Select-Object -First 1
    if ($null -eq $provider) {
        throw 'Active provider was missing from server settings.'
    }
    $provider.model = $model
    $provider.reasoningEffort = 'high'
    $provider.maxOutputTokens = 256
    $provider.rolloutBudget = $null
    $null = Invoke-OpenTopiaApi -Method 'PATCH' -Path '/api/settings' -Body @{
        providers = @($settings.providers)
        activeProviderId = $providerId
    }

    $stage = 'provider_capability_probe'
    $profile = Invoke-OpenTopiaApi -Method 'POST' -Path '/api/provider/test' -Body @{ providerId = $providerId }
    if ($profile.reachable -ne $true -or $profile.modelAvailable -ne $true) {
        throw 'Provider capability probe did not confirm the selected model.'
    }
    $refreshed = Invoke-OpenTopiaApi -Method 'GET' -Path '/api/settings'
    $refreshedProvider = @($refreshed.providers) | Where-Object { $_.id -eq $providerId } | Select-Object -First 1
    $profiles = $refreshedProvider.adapterProfiles
    $profilePresent = $null -eq $profiles -or $null -ne $profiles.PSObject.Properties[$model]
    if (-not $profilePresent) {
        throw 'Provider capability probe did not retain the model adapter profile.'
    }

    $stage = if ($ExerciseToolLoop) { 'tool_loop_turn' } else { 'full_tool_surface_turn' }
    $thread = Invoke-OpenTopiaApi -Method 'POST' -Path '/api/threads' -Body @{
        title = 'provider-wire-schema-smoke'
        workspaceRoot = '/tmp/opentopia-smoke-workspace'
    }
    if ([string]::IsNullOrWhiteSpace([string]$thread.id)) {
        throw 'Smoke thread was not created.'
    }
    $smokePrompt = if ($ExerciseToolLoop) {
        'Use an available file-editing tool to create exactly one file named smoke.txt containing OK in the workspace, then reply exactly with OK.'
    } else {
        'Reply exactly with OK. Do not call any tool.'
    }
    $null = Invoke-OpenTopiaApi -Method 'POST' -Path "/api/threads/$($thread.id)/messages" -Body @{
        content = $smokePrompt
    }
    $turn = $null
    $deadline = [DateTime]::UtcNow.AddSeconds($TurnTimeoutSeconds)
    while ([DateTime]::UtcNow -lt $deadline) {
        $candidate = Invoke-OpenTopiaApi -Method 'GET' -Path "/api/threads/$($thread.id)/turn"
        if ($candidate.status -in @('succeeded', 'failed', 'cancelled', 'interrupted', 'waiting_user_action')) {
            $turn = $candidate
            break
        }
        Start-Sleep -Seconds 1
    }
    if ($null -eq $turn) {
        throw 'Smoke turn exceeded its controlled timeout.'
    }
    if ($turn.status -ne 'succeeded') {
        throw 'Smoke turn did not succeed.'
    }

    $stage = 'write_report'
    $report = [ordered]@{
        schemaVersion = 1
        artifact = [IO.Path]::GetFileName($ServerBinary)
        model = $model
        reasoningEffort = 'high'
        maxOutputTokens = 256
        providerCapabilityProbe = [ordered]@{
            reachable = $true
            modelAvailable = $true
            adapterProfilePresent = $profilePresent
        }
        fullToolSurfaceSmoke = [ordered]@{
            turnStatus = [string]$turn.status
            providerErrorPresent = -not [string]::IsNullOrWhiteSpace([string]$turn.error)
            toolLoopRequested = [bool]$ExerciseToolLoop
        }
        notes = @(
            'The smoke used a minimal non-benchmark thread and did not inspect or store model text.',
            'No credential, authorization token, endpoint URL, thread ID, or response body is recorded.'
        )
    }
    New-Item -ItemType Directory -Force -Path (Split-Path -Parent $ReportPath) | Out-Null
    $report | ConvertTo-Json -Depth 10 | Set-Content -LiteralPath $ReportPath -Encoding utf8
    Write-Output 'Provider smoke passed.'
} catch {
    throw "Provider smoke failed at stage: $stage"
} finally {
    $apiKey = $null
    $baseUrl = $null
    $token = $null
    if ($containerStarted) {
        & docker.exe stop $containerName 1>$null 2>$null
    }
}
