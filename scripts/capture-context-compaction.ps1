param(
  [Parameter(Mandatory = $true)][string]$RunDirectory,
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [int]$Port = 8820,
  [string]$OutputPath = "docs\evaluations\context-compaction-long-horizon-capture.md",
  [ValidateRange(8192, 1048576)][int]$ContextWindowTokens = 32768,
  [ValidateRange(1024, 32768)][int]$MaxOutputTokens = 8192,
  [ValidateRange(1, 3)][int]$MaxAttempts = 3,
  [ValidateSet("", "none", "minimal", "low", "medium", "high")]
  [string]$ReasoningEffort = "low",
  [string]$ManualCheckpointPath = "",
  [switch]$InspectOnly
)

$ErrorActionPreference = "Stop"

function ConvertFrom-DotEnvFile {
  param([Parameter(Mandatory = $true)][string]$Path)

  $values = @{}
  Get-Content -LiteralPath $Path | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) {
      return
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

function Invoke-CaptureApi {
  param(
    [Parameter(Mandatory = $true)][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [AllowNull()][object]$Body = $null,
    [int]$TimeoutSeconds = 20
  )

  $parameters = @{
    Method = $Method
    Uri = "http://127.0.0.1:$Port$Path"
    Headers = $script:ApiHeaders
    TimeoutSec = $TimeoutSeconds
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json"
    $parameters.Body = $Body | ConvertTo-Json -Depth 30 -Compress
  }
  try {
    return Invoke-RestMethod @parameters
  } catch {
    if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
      throw "capture API request failed: $($_.ErrorDetails.Message)"
    }
    $response = $_.Exception.Response
    if ($response) {
      $details = $null
      try {
        $reader = New-Object IO.StreamReader($response.GetResponseStream())
        $details = $reader.ReadToEnd()
        $reader.Dispose()
      } catch {
        $details = $null
      }
      if ($details) {
        throw "capture API request failed: $details"
      }
    }
    throw
  }
}

function Wait-ForHealth {
  param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process)

  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    if ($Process.HasExited) {
      throw "capture server exited before becoming healthy"
    }
    Start-Sleep -Milliseconds 250
    try {
      $health = Invoke-CaptureApi "Get" "/health" -TimeoutSeconds 2
      if ($health.ok) {
        return
      }
    } catch {
    }
  }
  throw "capture server did not become healthy"
}

function Get-RequestStats {
  param([AllowNull()][object]$Event)

  if ($null -eq $Event) {
    return $null
  }
  $request = $Event.payload.request
  $json = $request | ConvertTo-Json -Depth 100
  return [ordered]@{
    seq = $Event.seq
    requestId = $Event.payload.request_id
    round = $Event.payload.round
    jsonCharacters = $json.Length
    roughJsonTokens = [Math]::Ceiling($json.Length / 4)
    contextItems = @($request.contextItems).Count
    conversationMessages = @($request.conversation).Count
    previousToolCalls = @($request.previousToolCalls).Count
    toolResults = @($request.toolResults).Count
    toolCandidates = @($request.toolCandidates).Count
    userMessageCharacters = ([string]$request.userMessage).Length
  }
}

function Convert-ToJsonBlock {
  param([AllowNull()][object]$Value)

  if ($null -eq $Value) {
    return "_not captured_"
  }
  $json = $Value | ConvertTo-Json -Depth 100
  return ('`````json' + "`n" + $json + "`n" + '`````')
}

$repoRoot = if ($PSScriptRoot) {
  Split-Path -Parent $PSScriptRoot
} else {
  [IO.Path]::GetFullPath((Get-Location).Path)
}
$runRoot = if ([IO.Path]::IsPathRooted($RunDirectory)) {
  [IO.Path]::GetFullPath($RunDirectory)
} else {
  [IO.Path]::GetFullPath((Join-Path $repoRoot $RunDirectory))
}
$database = Join-Path $runRoot "evaluation.db"
$sourceTrajectoryPath = Join-Path $runRoot "trajectory-final.json"
$resultPath = Join-Path $runRoot "result.json"
if (-not (Test-Path -LiteralPath $database -PathType Leaf)) {
  throw "evaluation database not found: $database"
}
if (-not (Test-Path -LiteralPath $sourceTrajectoryPath -PathType Leaf)) {
  throw "source trajectory not found: $sourceTrajectoryPath"
}

$values = ConvertFrom-DotEnvFile $EnvFile
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "selected provider profile is incomplete"
}

$sourceTrajectory = Get-Content -Raw -Encoding UTF8 -LiteralPath $sourceTrajectoryPath |
  ConvertFrom-Json
$threadId = [string]$sourceTrajectory.thread.id
if (-not $threadId) {
  throw "source trajectory has no thread id"
}
$runResult = if (Test-Path -LiteralPath $resultPath) {
  Get-Content -Raw -Encoding UTF8 -LiteralPath $resultPath | ConvertFrom-Json
} else {
  $null
}

$serverPath = Join-Path $repoRoot ".opentopia\verify-target\debug\opentopia-server.exe"
if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
  throw "evaluation server binary not found: $serverPath"
}

$env:OPENTOPIA_API_KEY = $apiKey
$env:OPENTOPIA_OPENAI_BASE_URL = $baseUrl
$env:OPENTOPIA_MODEL = $model
$env:OPENTOPIA_API_TOKEN = "opentopia-local-verification-token-0123456789abcdef0123456789abcdef"
$env:OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT = "50"
$env:OPENTOPIA_SANDBOX_MODE = "workspace-write"
$env:OPENTOPIA_SANDBOX_ENFORCEMENT = "best-effort"
$env:OPENTOPIA_SANDBOX_NETWORK = "deny"
$script:ApiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }

$stdoutPath = Join-Path $runRoot "server-capture.stdout.log"
$stderrPath = Join-Path $runRoot "server-capture.stderr.log"
$server = Start-Process `
  -FilePath $serverPath `
  -ArgumentList @("--port", $Port, "--db", $database, "--permission", "full-access") `
  -RedirectStandardOutput $stdoutPath `
  -RedirectStandardError $stderrPath `
  -PassThru `
  -WindowStyle Hidden

try {
  Wait-ForHealth $server
  if (-not $InspectOnly) {
    $settings = Invoke-CaptureApi "Get" "/api/settings"
    $activeProvider = @($settings.providers | Where-Object {
        $_.id -eq $settings.activeProviderId
      })[0]
    if (-not $activeProvider) {
      throw "active provider is missing from settings"
    }
    $activeProvider.baseUrl = $baseUrl
    $activeProvider.model = $model
    $activeProvider.contextWindowTokens = $ContextWindowTokens
    $activeProvider.maxOutputTokens = $MaxOutputTokens
    $activeProvider.reasoningEffort = $ReasoningEffort
    Invoke-CaptureApi "Patch" "/api/settings" @{
      providers = @($settings.providers)
      activeProviderId = $settings.activeProviderId
    } | Out-Null
  }
  $eventsBefore = Invoke-CaptureApi "Get" "/api/threads/$threadId/events"
  $beforeMaxSeq = [int64](($eventsBefore |
      Measure-Object -Property seq -Maximum).Maximum)

  if ($InspectOnly) {
    $trajectory = Invoke-CaptureApi "Get" "/api/threads/$threadId/trajectory" -TimeoutSeconds 30
    $roundZeroRequests = @($trajectory.events | Where-Object {
        $_.payload.type -eq "model_request" -and $_.payload.round -eq 0
      })
    $roundZeroResponses = @($trajectory.events | Where-Object {
        $_.payload.type -eq "provider_response_received" -and $_.payload.round -eq 0
      })
    [PSCustomObject]@{
      roundZeroRequestCount = $roundZeroRequests.Count
      roundZeroRequests = @($roundZeroRequests | ForEach-Object {
          $requestJson = $_.payload.request | ConvertTo-Json -Depth 100 -Compress
          [PSCustomObject]@{
            seq = $_.seq
            jsonCharacters = $requestJson.Length
            roughJsonTokens = [Math]::Ceiling($requestJson.Length / 4)
            userMessageCharacters = ([string]$_.payload.request.userMessage).Length
          }
        })
      roundZeroResponses = @($roundZeroResponses | ForEach-Object {
          $responseJson = $_.payload.body | ConvertTo-Json -Depth 100 -Compress
          $previewLength = [Math]::Min(1000, $responseJson.Length)
          [PSCustomObject]@{
            seq = $_.seq
            status = $_.payload.status
            bodyCharacters = $responseJson.Length
            bodyPreview = $responseJson.Substring(0, $previewLength)
          }
        })
    } | ConvertTo-Json -Depth 10
    return
  }

  $summary = $null
  $compactError = $null
  $compactBody = @{}
  if ($ManualCheckpointPath) {
    $resolvedCheckpointPath = if ([IO.Path]::IsPathRooted($ManualCheckpointPath)) {
      [IO.Path]::GetFullPath($ManualCheckpointPath)
    } else {
      [IO.Path]::GetFullPath((Join-Path $repoRoot $ManualCheckpointPath))
    }
    $manualCheckpoint = Get-Content `
      -LiteralPath $resolvedCheckpointPath `
      -Raw `
      -Encoding UTF8 | ConvertFrom-Json
    $compactBody = @{ checkpoint = $manualCheckpoint }
  }
  for ($attempt = 1; $attempt -le $MaxAttempts -and $null -eq $summary; $attempt += 1) {
    try {
      $summary = Invoke-CaptureApi `
        -Method "Post" `
        -Path "/api/threads/$threadId/context/compact" `
        -Body $compactBody `
        -TimeoutSeconds 120
    } catch {
      $compactError = $_.Exception.Message
      if ($attempt -lt $MaxAttempts) {
        Start-Sleep -Seconds 30
      }
    }
  }
  if ($null -eq $summary) {
    throw "context compaction failed after $MaxAttempts attempt(s): $compactError"
  }

  $eventsAfterCompact = Invoke-CaptureApi "Get" "/api/threads/$threadId/events"
  $compactionEvent = $eventsAfterCompact |
    Where-Object {
      $_.payload.type -eq "context_compacted" -and $_.seq -gt $beforeMaxSeq
    } |
    Select-Object -Last 1
  if ($null -eq $compactionEvent) {
    throw "context compaction succeeded but no ContextCompacted event was found"
  }

  $capturePrompt = @"
Continue from the durable checkpoint. This is a context-recovery capture turn.
Do not modify files or execute tools. Reply with one short sentence stating the current task status.
"@
  $captureMessage = Invoke-CaptureApi `
    -Method "Post" `
    -Path "/api/threads/$threadId/messages" `
    -Body @{ content = $capturePrompt }

  $afterRequest = $null
  $deadline = (Get-Date).AddSeconds(60)
  while ((Get-Date) -lt $deadline -and $null -eq $afterRequest) {
    Start-Sleep -Milliseconds 500
    $events = Invoke-CaptureApi "Get" "/api/threads/$threadId/events"
    $afterRequest = $events |
      Where-Object {
        $_.payload.type -eq "model_request" -and
        $_.payload.round -gt 0 -and
        $_.seq -gt $compactionEvent.seq
      } |
      Select-Object -First 1
  }
  if ($null -eq $afterRequest) {
    throw "no model request was captured after compaction"
  }
  try {
    Invoke-CaptureApi `
      -Method "Post" `
      -Path "/api/threads/$threadId/turn/cancel" `
      -Body @{} | Out-Null
  } catch {
  }

  $trajectory = Invoke-CaptureApi "Get" "/api/threads/$threadId/trajectory" -TimeoutSeconds 30
  $trajectoryJson = $trajectory | ConvertTo-Json -Depth 100
  if ($trajectoryJson.Contains($apiKey)) {
    throw "secret audit rejected captured trajectory"
  }
  $captureTrajectoryPath = Join-Path $runRoot "trajectory-context-capture.json"
  [IO.File]::WriteAllText(
    $captureTrajectoryPath,
    "$trajectoryJson`n",
    [Text.UTF8Encoding]::new($false)
  )

  $events = @($trajectory.events)
  $compactionEvent = $events |
    Where-Object { $_.payload.type -eq "context_compacted" } |
    Select-Object -Last 1
  $compactionRequest = $events |
    Where-Object {
      $_.payload.type -eq "model_request" -and
      $_.payload.round -eq 0 -and
      $_.seq -lt $compactionEvent.seq
    } |
    Select-Object -Last 1
  $compactionResponse = $events |
    Where-Object {
      $_.payload.type -eq "provider_response_received" -and
      $_.payload.request_id -eq $compactionRequest.payload.request_id
    } |
    Select-Object -Last 1
  $beforeRequest = $events |
    Where-Object {
      $_.payload.type -eq "model_request" -and
      $_.payload.round -gt 0 -and
      $_.seq -lt $compactionRequest.seq
    } |
    Select-Object -Last 1
  $afterRequest = $events |
    Where-Object {
      $_.payload.type -eq "model_request" -and
      $_.payload.round -gt 0 -and
      $_.seq -gt $compactionEvent.seq
    } |
    Select-Object -First 1

  $beforeStats = Get-RequestStats $beforeRequest
  $compactionStats = Get-RequestStats $compactionRequest
  $afterStats = Get-RequestStats $afterRequest
  $checkpoint = $compactionEvent.payload.summary.checkpoint
  $checkpointJson = $checkpoint | ConvertTo-Json -Depth 100
  $compactionMetadata = $compactionEvent.payload.summary.metadata
  $reportDate = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss zzz")
  $sourceStatus = if ($runResult) { [string]$runResult.status } else { "unknown" }
  $sourceTokens = if ($runResult) {
    [int64]$runResult.trajectoryMetrics.totalTokens
  } else { 0 }
  $sourceTurns = if ($runResult) { @($runResult.turns).Count } else { 0 }
  $sourceModel = if ($runResult -and $runResult.provider.model) {
    [string]$runResult.provider.model
  } else { $model }
  $automaticAttemptCount = @($events | Where-Object {
      $_.payload.type -eq "model_request" -and $_.payload.round -eq 0
    }).Count
  $automaticResponseCount = @($events | Where-Object {
      $_.payload.type -eq "provider_response_received" -and $_.payload.round -eq 0
    }).Count
  $automaticUsage = $compactionResponse.payload.body.usage
  $requestReductionPercent = if ($beforeStats.jsonCharacters -gt 0) {
    [Math]::Round(
      (($beforeStats.jsonCharacters - $afterStats.jsonCharacters) * 100.0) /
        $beforeStats.jsonCharacters,
      2
    )
  } else { 0 }
  $beforeContextIds = @($beforeRequest.payload.request.contextItems | ForEach-Object { $_.id })
  $afterContextIds = @($afterRequest.payload.request.contextItems | ForEach-Object { $_.id })
  $preservedContextItemCount = @($beforeContextIds | Where-Object {
      $_ -in $afterContextIds
    }).Count
  $removedConversationCount = [Math]::Max(
    0,
    $beforeStats.conversationMessages - $afterStats.conversationMessages
  )
  $afterCheckpointItem = $afterRequest.payload.request.contextItems |
    Where-Object { $_.kind -eq "checkpoint" } |
    Select-Object -First 1
  $manualMode = [string]$checkpoint.mode -eq "manual"
  $compactionNarrative = if ($manualMode) {
    "事件中累计记录 $automaticAttemptCount 个 round-0 自动摘要请求，其中 $automaticResponseCount 个收到 Provider 响应；其余是本地 Ollama 未运行时的传输失败。最后一次 GLM 请求输入 $($automaticUsage.inputTokens) token，输出上限 $($automaticUsage.outputTokens) token 全部被计为 reasoning，正文为 $($compactionResponse.payload.body.textChars) 字符，因此没有产生可解析的 checkpoint。为完成前后对照，最终通过正式结构化 checkpoint API 提交人工草稿；服务端仍执行 schema、source seq、活动计划状态、预算、coverage 和 lineage 校验。这里不能把最终 checkpoint 称为 GLM 生成结果。"
  } else {
    "自动摘要成功生成结构化 checkpoint。事件中累计记录 $automaticAttemptCount 个 round-0 请求，其中 $automaticResponseCount 个收到 Provider 响应；本次成功请求输入 $($automaticUsage.inputTokens) token，输出 $($automaticUsage.outputTokens) token，正文为 $($compactionResponse.payload.body.textChars) 字符。"
  }
  $metadataCaveat = if ($manualMode) {
    "手动模式下，事实/约束保留率为入口元数据的 100%，不是独立评测分数；tokenReductionPercent 也固定为 0，不能拿来表示真实请求缩减。"
  } else {
    "自动模式的 tokenReductionPercent 比较压缩 snapshot 与 checkpoint，不等同于整个模型请求的缩减比例。"
  }
  $compactionRequestHeading = if ($manualMode) {
    "## 5. 自动压缩模型收到的请求（完整、已脱敏；本次失败）"
  } else {
    "## 5. 自动压缩模型收到的请求（完整、已脱敏）"
  }
  $compactionRequestDescription = if ($manualMode) {
    "该请求的 userMessage 是增量 snapshot；finalOutputJsonSchema 要求模型返回结构化 checkpoint delta。GLM 返回 finishReason=length，$($automaticUsage.outputTokens) 个输出 token 全为 reasoning，正文为 0 字符。"
  } else {
    "该请求的 userMessage 是增量 snapshot；finalOutputJsonSchema 要求模型返回结构化 checkpoint delta。"
  }
  $checkpointHeading = if ($manualMode) {
    "## 6. 最终采用的手动结构化 checkpoint（完整）"
  } else {
    "## 6. 自动压缩输出 checkpoint（完整）"
  }
  $checkpointDescription = if ($manualMode) {
    "checkpoint 内容来自人工草稿；coverage、ID、lineage、mode 和 compatibility hash 由服务端生成或校验。"
  } else {
    "checkpoint 的 coverage、ID、lineage、mode 和 compatibility hash 由服务端控制；模型生成内容字段。"
  }

  $report = @"
# OpenTopia 长程任务上下文压缩前后对照

生成时间：$reportDate  
长程任务模型：$sourceModel  
自动压缩尝试模型：$model  
线程：$threadId  
源评测：$($runRoot.Replace($repoRoot + '\', ''))  
源评测状态：$sourceStatus（这是任务执行结果，不代表压缩是否成功）  
源轨迹：$sourceTurns 个阶段，累计 $sourceTokens token  

## 1. 测试说明

本报告先运行两阶段 ledger 长程编码任务并重启服务，再对同一线程执行上下文压缩，最后创建一个最小恢复轮并在其首个 model_request 写入事件日志后取消。

源长程任务没有完成：GLM relay 在阶段一返回一次错误请求，并在后续命中 RPM 限制；但是 SQLite 事件、消息、计划和重启恢复轨迹均保留。本报告评估的是“已有长轨迹如何被压缩并重建下一次请求”，不是该编码任务的成功率。

$compactionNarrative

## 2. 压缩结果

| 项目 | 数值 |
| --- | ---: |
| 成功压缩模式 | $($checkpoint.mode) |
| 压缩来源 | $($compactionMetadata.source) |
| 压缩事件 seq | $($compactionEvent.seq) |
| 覆盖到事件 seq | $($compactionEvent.payload.summary.coveredThroughSeq) |
| 覆盖消息数 | $($checkpoint.coverage.throughMessageCount) |
| 自动摘要 request seq | $($compactionStats.seq) |
| 自动摘要 Provider 输入 token | $($automaticUsage.inputTokens) |
| 自动摘要输出 / reasoning token | $($automaticUsage.outputTokens) / $($automaticUsage.reasoningTokens) |
| 自动摘要正文字符数 | $($compactionResponse.payload.body.textChars) |
| checkpoint 预算 token | $($compactionMetadata.checkpointBudgetTokens) |
| checkpoint token 估算 | $($compactionMetadata.checkpointTokens) |
| 事实保留率 | $($compactionMetadata.factRetentionPercent)% |
| 活跃约束保留率 | $($compactionMetadata.activeConstraintRetentionPercent)% |
| 压缩耗时 | $($compactionMetadata.latencyMs) ms |

$metadataCaveat

## 3. 压缩前后请求尺寸

| 项目 | 压缩前 | 压缩后 |
| --- | ---: | ---: |
| event seq | $($beforeStats.seq) | $($afterStats.seq) |
| round | $($beforeStats.round) | $($afterStats.round) |
| 请求 JSON 字符数 | $($beforeStats.jsonCharacters) | $($afterStats.jsonCharacters) |
| 请求 JSON 粗略 token | $($beforeStats.roughJsonTokens) | $($afterStats.roughJsonTokens) |
| 整体请求字符缩减 | - | $requestReductionPercent% |
| context items | $($beforeStats.contextItems) | $($afterStats.contextItems) |
| conversation messages | $($beforeStats.conversationMessages) | $($afterStats.conversationMessages) |
| previous tool calls | $($beforeStats.previousToolCalls) | $($afterStats.previousToolCalls) |
| tool results | $($beforeStats.toolResults) | $($afterStats.toolResults) |
| tool candidates | $($beforeStats.toolCandidates) | $($afterStats.toolCandidates) |
| user message 字符数 | $($beforeStats.userMessageCharacters) | $($afterStats.userMessageCharacters) |

注意：请求 JSON 的粗略 token 只是字符数除以 4，不等同于 Provider tokenizer。压缩元数据中的 input/checkpoint token 使用 OpenTopia 的 Unicode-aware 估算器。

具体结构变化：

- 前后有 $preservedContextItemCount 个 context item ID 完全相同；基础契约、developer/repository/environment/skill 等稳定项被保留。
- conversation 从 $($beforeStats.conversationMessages) 条降到 $($afterStats.conversationMessages) 条，移除 $removedConversationCount 条较早历史。
- 压缩后注入 checkpoint context item，估算 $($afterCheckpointItem.tokenEstimate) token；同一 durable checkpoint 还会被包装进当前 userMessage，解释了 userMessage 字符数反而上升。
- tool candidates 前后均为 $($afterStats.toolCandidates)，说明工具 schema 不属于被压缩的历史会话。

## 4. 压缩前模型请求（完整、已脱敏）

$(Convert-ToJsonBlock $beforeRequest.payload.request)

$compactionRequestHeading

$compactionRequestDescription

$(Convert-ToJsonBlock $compactionRequest.payload.request)

$checkpointHeading

$checkpointDescription

$(Convert-ToJsonBlock $checkpoint)

## 7. 压缩后第一次模型请求（完整、已脱敏）

该请求应保留稳定 system/developer context，把 checkpoint 作为 earlier-turn durable context 注入，并只重放有界 recent tail。

$(Convert-ToJsonBlock $afterRequest.payload.request)

## 8. 原始证据

- 压缩前轨迹：$($sourceTrajectoryPath.Replace($repoRoot + '\', ''))
- 含压缩后恢复请求的轨迹：$($captureTrajectoryPath.Replace($repoRoot + '\', ''))
- 结构化 checkpoint ID：$($checkpoint.id)
- 压缩前 request ID：$($beforeRequest.payload.request_id)
- 压缩请求 ID：$($compactionRequest.payload.request_id)
- 压缩后 request ID：$($afterRequest.payload.request_id)
"@

  if ($report.Contains($apiKey)) {
    throw "secret audit rejected generated report"
  }
  $resolvedOutput = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
  } else {
    [IO.Path]::GetFullPath((Join-Path $repoRoot $OutputPath))
  }
  $outputParent = Split-Path -Parent $resolvedOutput
  New-Item -ItemType Directory -Path $outputParent -Force | Out-Null
  [IO.File]::WriteAllText(
    $resolvedOutput,
    "$report`n",
    [Text.UTF8Encoding]::new($false)
  )

  [PSCustomObject]@{
    outputPath = $resolvedOutput
    trajectoryPath = $captureTrajectoryPath
    threadId = $threadId
    compactionSeq = $compactionEvent.seq
    checkpointId = $checkpoint.id
    inputTokens = $compactionMetadata.inputTokens
    checkpointTokens = $compactionMetadata.checkpointTokens
    tokenReductionPercent = $compactionMetadata.tokenReductionPercent
    beforeRequestCharacters = $beforeStats.jsonCharacters
    afterRequestCharacters = $afterStats.jsonCharacters
  } | ConvertTo-Json -Depth 10
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force
    $server.WaitForExit(5000) | Out-Null
  }
}
