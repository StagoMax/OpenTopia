param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "",
  [string]$OutputPath = ""
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  if (-not (Test-Path -LiteralPath $Path)) {
    throw "Provider env file was not found"
  }

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) {
      return
    }
    if ($line.StartsWith("export ")) {
      $line = $line.Substring(7).Trim()
    }
    $parts = $line.Split("=", 2)
    $value = $parts[1].Trim()
    if (
      $value.Length -ge 2 -and
      (($value.StartsWith('"') -and $value.EndsWith('"')) -or
        ($value.StartsWith("'") -and $value.EndsWith("'")))
    ) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    $values[$parts[0].Trim()] = $value
  }
  return $values
}

function Protect-ProviderError {
  param(
    [AllowNull()][string]$Message,
    [Parameter(Mandatory = $true)][string]$ApiKey
  )

  if ([string]::IsNullOrWhiteSpace($Message)) {
    return $null
  }
  $safe = $Message.Replace($ApiKey, "<redacted>")
  $safe = $safe -replace '(?i)Bearer\s+[A-Za-z0-9._~+/=-]+', 'Bearer <redacted>'
  $safe = $safe -replace '(?i)(api[_-]?key["''\s:=]+)[A-Za-z0-9._~+/=-]+', '$1<redacted>'
  if ($safe.Length -gt 500) {
    $safe = $safe.Substring(0, 500)
  }
  return $safe
}

function Invoke-JsonPost {
  param(
    [Parameter(Mandatory = $true)][System.Net.Http.HttpClient]$Client,
    [Parameter(Mandatory = $true)][string]$Url,
    [Parameter(Mandatory = $true)][object]$Payload
  )

  $json = $Payload | ConvertTo-Json -Depth 40 -Compress
  $content = [System.Net.Http.StringContent]::new(
    $json,
    [Text.Encoding]::UTF8,
    "application/json"
  )
  $response = $Client.PostAsync($Url, $content).GetAwaiter().GetResult()
  $body = $response.Content.ReadAsStringAsync().GetAwaiter().GetResult()
  return [PSCustomObject]@{
    Status = [int]$response.StatusCode
    Success = $response.IsSuccessStatusCode
    ContentType = [string]$response.Content.Headers.ContentType
    Body = $body
  }
}

function ConvertFrom-SseBody {
  param([Parameter(Mandatory = $true)][string]$Body)

  $events = @()
  $invalidEvents = 0
  $done = $false
  foreach ($line in ($Body -split "`r?`n")) {
    if (-not $line.StartsWith("data:")) {
      continue
    }
    $data = $line.Substring(5).Trim()
    if ($data -eq "[DONE]") {
      $done = $true
      continue
    }
    if (-not $data) {
      continue
    }
    try {
      # Windows PowerShell 5.1 does not support ConvertFrom-Json -Depth.
      $events += ($data | ConvertFrom-Json)
    } catch {
      $invalidEvents += 1
    }
  }
  return [PSCustomObject]@{
    Events = $events
    InvalidEvents = $invalidEvents
    Done = $done
  }
}

function Measure-StreamResponse {
  param(
    [Parameter(Mandatory = $true)][object]$Response,
    [Parameter(Mandatory = $true)][string]$ApiKey
  )

  $sse = ConvertFrom-SseBody $Response.Body
  $contentDelta = @($sse.Events | Where-Object {
    $null -ne $_.choices[0].delta.content
  }).Count -gt 0
  $usage = @($sse.Events | Where-Object { $null -ne $_.usage }).Count -gt 0
  return [ordered]@{
    status = $Response.Status
    contentType = $Response.ContentType
    bodyBytes = [Text.Encoding]::UTF8.GetByteCount($Response.Body)
    sseEvents = $sse.Events.Count
    invalidSseEvents = $sse.InvalidEvents
    done = $sse.Done
    contentDelta = $contentDelta
    usage = $usage
    finishReasons = @($sse.Events | ForEach-Object {
      $_.choices[0].finish_reason
    } | Where-Object { $_ } | Select-Object -Unique)
    error = if ($Response.Success) {
      $null
    } else {
      Protect-ProviderError $Response.Body $ApiKey
    }
  }
}

$values = ConvertFrom-DotEnvFile $EnvFile
$apiKeyName = "${Profile}_API_KEY"
$baseUrlName = "${Profile}_BASE_URL"
$modelName = "${Profile}_MODEL"
$apiKey = [string]$values[$apiKeyName]
$baseUrl = ([string]$values[$baseUrlName]).TrimEnd("/")
$model = [string]$values[$modelName]
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "The selected provider profile is incomplete"
}
if ($ExpectedModel -and $model -ne $ExpectedModel) {
  throw "Selected model does not match the expected model"
}

Add-Type -AssemblyName System.Net.Http
$client = [System.Net.Http.HttpClient]::new()
$client.Timeout = [TimeSpan]::FromSeconds(90)
$client.DefaultRequestHeaders.Authorization =
  [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $apiKey)

try {
  $result = [ordered]@{
    schemaVersion = 1
    checkedAt = (Get-Date).ToUniversalTime().ToString("o")
    profile = $Profile
    baseUrl = $baseUrl
    model = $model
    credentials = "redacted:set"
    models = [ordered]@{}
    streamChat = [ordered]@{}
    streamToolsAuto = [ordered]@{}
    streamToolContinuation = [ordered]@{}
    streamSerializedToolHistory = [ordered]@{}
    streamCompactedToolHistory = [ordered]@{}
    streamToolsForced = [ordered]@{}
    compatibleWithOpenTopia = $false
  }

  try {
    $modelsResponse = $client.GetAsync("$baseUrl/models").GetAwaiter().GetResult()
    $modelsBody = $modelsResponse.Content.ReadAsStringAsync().GetAwaiter().GetResult()
    $schema = "unrecognized"
    $topLevelKeys = @()
    $modelIds = @()
    try {
      $modelsJson = $modelsBody | ConvertFrom-Json
      $topLevelKeys = @($modelsJson.PSObject.Properties.Name)
      if ($null -ne $modelsJson.data) {
        $schema = "object:data[]"
        $modelIds = @($modelsJson.data | ForEach-Object { $_.id })
      } elseif ($modelsJson -is [array]) {
        $schema = "array"
        $modelIds = @($modelsJson | ForEach-Object { $_.id })
      }
    } catch {
    }
    $result.models = [ordered]@{
      status = [int]$modelsResponse.StatusCode
      contentType = [string]$modelsResponse.Content.Headers.ContentType
      schema = $schema
      topLevelKeys = $topLevelKeys
      modelCount = $modelIds.Count
      modelListed = $model -in $modelIds
      error = if ($modelsResponse.IsSuccessStatusCode) {
        $null
      } else {
        Protect-ProviderError $modelsResponse.ReasonPhrase $apiKey
      }
    }
  } catch {
    $result.models = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
  }

  $chatPayload = @{
    model = $model
    messages = @(@{ role = "user"; content = "Reply with exactly PROBE_OK." })
    temperature = 0
    max_tokens = 32
    stream = $true
    stream_options = @{ include_usage = $true }
  }
  try {
    $chatResponse = Invoke-JsonPost $client "$baseUrl/chat/completions" $chatPayload
    $result.streamChat = Measure-StreamResponse $chatResponse $apiKey
  } catch {
    $result.streamChat = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
  }

  $toolDefinition = @{
    type = "function"
    function = @{
      name = "eval_probe"
      description = "Protocol compatibility probe"
      parameters = @{
        type = "object"
        properties = @{
          value = @{ type = "string"; description = "Probe value" }
        }
        required = @("value")
      }
    }
  }
  $toolPayload = @{
    model = $model
    messages = @(
      @{ role = "system"; content = "You are a tool-using coding agent." },
      @{ role = "user"; content = "Use eval_probe with value ready, then report success." }
    )
    temperature = 0.2
    stream = $true
    stream_options = @{ include_usage = $true }
    tools = @($toolDefinition)
    tool_choice = "auto"
    parallel_tool_calls = $false
  }
  try {
    $toolResponse = Invoke-JsonPost $client "$baseUrl/chat/completions" $toolPayload
    $toolMetric = Measure-StreamResponse $toolResponse $apiKey
    $toolSse = ConvertFrom-SseBody $toolResponse.Body
    $toolDeltas = @($toolSse.Events | ForEach-Object {
      $_.choices[0].delta.tool_calls
    } | Where-Object { $null -ne $_ })
    $argumentsText = ($toolDeltas | ForEach-Object {
      $_.function.arguments
    }) -join ""
    $argumentsJson = $false
    if ($argumentsText) {
      try {
        $arguments = $argumentsText | ConvertFrom-Json
        $argumentsJson = $arguments.value -eq "ready"
      } catch {
      }
    }
    $toolMetric.toolName = @($toolDeltas | Where-Object {
      $_.function.name -eq "eval_probe"
    }).Count -gt 0
    $toolMetric.argumentsJson = $argumentsJson
    $result.streamToolsAuto = $toolMetric

    $callId = @($toolDeltas | ForEach-Object { $_.id } |
      Where-Object { $_ } | Select-Object -First 1)[0]
    $toolName = @($toolDeltas | ForEach-Object { $_.function.name } |
      Where-Object { $_ } | Select-Object -First 1)[0]
    if ($callId -and $toolName -and $argumentsJson) {
      $continuationPayload = @{
        model = $model
        messages = @(
          @{ role = "system"; content = "You are a tool-using coding agent." },
          @{ role = "user"; content = "Use eval_probe exactly once with value ready, then report success." },
          @{
            role = "assistant"
            content = ""
            tool_calls = @(@{
              id = $callId
              type = "function"
              function = @{ name = $toolName; arguments = $argumentsText }
            })
          },
          @{
            role = "tool"
            tool_call_id = $callId
            content = '{"output":"ready","isError":false}'
          }
        )
        temperature = 0.2
        stream = $true
        stream_options = @{ include_usage = $true }
        tools = @($toolDefinition)
        tool_choice = "auto"
        parallel_tool_calls = $false
      }
      $continuationResponse = Invoke-JsonPost `
        $client `
        "$baseUrl/chat/completions" `
        $continuationPayload
      $result.streamToolContinuation =
        Measure-StreamResponse $continuationResponse $apiKey

      $serializedPayload = @{
        model = $model
        messages = @(
          @{ role = "system"; content = "You are a tool-using coding agent." },
          @{ role = "user"; content = "Acknowledge both completed probe calls." },
          @{
            role = "assistant"
            content = ""
            tool_calls = @(@{
              id = "serialized_probe_1"
              type = "function"
              function = @{ name = "eval_probe"; arguments = '{"value":"first"}' }
            })
          },
          @{
            role = "tool"
            tool_call_id = "serialized_probe_1"
            content = '{"output":"first","isError":false}'
          },
          @{
            role = "assistant"
            content = ""
            tool_calls = @(@{
              id = "serialized_probe_2"
              type = "function"
              function = @{ name = "eval_probe"; arguments = '{"value":"second"}' }
            })
          },
          @{
            role = "tool"
            tool_call_id = "serialized_probe_2"
            content = '{"output":"second","isError":false}'
          }
        )
        temperature = 0.2
        stream = $true
        stream_options = @{ include_usage = $true }
        tools = @($toolDefinition)
        tool_choice = "auto"
        parallel_tool_calls = $false
      }
      $serializedResponse = Invoke-JsonPost `
        $client `
        "$baseUrl/chat/completions" `
        $serializedPayload
      $result.streamSerializedToolHistory =
        Measure-StreamResponse $serializedResponse $apiKey

      $compactedPayload = @{
        model = $model
        messages = @(
          @{ role = "system"; content = "You are a tool-using coding agent." },
          @{ role = "user"; content = "Continue after the completed tool history." },
          @{
            role = "user"
            content = 'Completed tool history: [{"name":"eval_probe","arguments":{"value":"first"},"output":"first"},{"name":"eval_probe","arguments":{"value":"second"},"output":"second"}]'
          }
        )
        temperature = 0.2
        stream = $true
        stream_options = @{ include_usage = $true }
        tools = @($toolDefinition)
        tool_choice = "auto"
        parallel_tool_calls = $false
      }
      $compactedResponse = Invoke-JsonPost `
        $client `
        "$baseUrl/chat/completions" `
        $compactedPayload
      $result.streamCompactedToolHistory =
        Measure-StreamResponse $compactedResponse $apiKey
    } else {
      $result.streamToolContinuation = [ordered]@{
        status = $null
        error = "initial tool call was incomplete"
      }
      $result.streamSerializedToolHistory = [ordered]@{
        status = $null
        error = "initial tool call was incomplete"
      }
      $result.streamCompactedToolHistory = [ordered]@{
        status = $null
        error = "initial tool call was incomplete"
      }
    }
  } catch {
    $result.streamToolsAuto = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
    $result.streamToolContinuation = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
    $result.streamSerializedToolHistory = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
    $result.streamCompactedToolHistory = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
  }

  $forcedPayload = $toolPayload.Clone()
  $forcedPayload.tool_choice = @{
    type = "function"
    function = @{ name = "eval_probe" }
  }
  try {
    $forcedResponse = Invoke-JsonPost $client "$baseUrl/chat/completions" $forcedPayload
    $forcedMetric = Measure-StreamResponse $forcedResponse $apiKey
    $forcedSse = ConvertFrom-SseBody $forcedResponse.Body
    $forcedToolDeltas = @($forcedSse.Events | ForEach-Object {
      $_.choices[0].delta.tool_calls
    } | Where-Object { $null -ne $_ })
    $forcedMetric.toolName = @($forcedToolDeltas | Where-Object {
      $_.function.name -eq "eval_probe"
    }).Count -gt 0
    $result.streamToolsForced = $forcedMetric
  } catch {
    $result.streamToolsForced = [ordered]@{
      status = $null
      error = Protect-ProviderError $_.Exception.Message $apiKey
    }
  }

  $result.compatibleWithOpenTopia =
    $result.models.status -ge 200 -and
    $result.models.status -lt 300 -and
    $result.models.modelListed -and
    $result.streamChat.status -ge 200 -and
    $result.streamChat.status -lt 300 -and
    $result.streamChat.contentDelta -and
    $result.streamChat.invalidSseEvents -eq 0 -and
    $result.streamToolsAuto.status -ge 200 -and
    $result.streamToolsAuto.status -lt 300 -and
    $result.streamToolsAuto.toolName -and
    $result.streamToolsAuto.argumentsJson -and
    $result.streamToolContinuation.status -ge 200 -and
    $result.streamToolContinuation.status -lt 300 -and
    $result.streamToolContinuation.invalidSseEvents -eq 0 -and
    $result.streamCompactedToolHistory.status -ge 200 -and
    $result.streamCompactedToolHistory.status -lt 300 -and
    $result.streamCompactedToolHistory.invalidSseEvents -eq 0

  $json = $result | ConvertTo-Json -Depth 20
  if ($json.Contains($apiKey)) {
    throw "Secret audit rejected provider probe output"
  }
  if ($OutputPath) {
    $parent = Split-Path -Parent $OutputPath
    if ($parent) {
      New-Item -ItemType Directory -Path $parent -Force | Out-Null
    }
    [IO.File]::WriteAllText($OutputPath, "$json`n", [Text.UTF8Encoding]::new($false))
  }
  $json
} finally {
  $client.Dispose()
}
