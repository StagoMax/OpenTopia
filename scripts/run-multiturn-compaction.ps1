param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [int]$Port = 8823,
  [ValidateRange(4, 20)][int]$TurnCount = 8,
  [ValidateRange(8192, 1048576)][int]$ContextWindowTokens = 32768,
  [ValidateRange(1024, 32768)][int]$MaxOutputTokens = 4096,
  [string]$OutputPath = "docs\evaluations\context-compaction-multiturn-glm-2026-07-28.md",
  [switch]$UseMockProvider
)

$ErrorActionPreference = "Stop"

function Read-DotEnv {
  param([Parameter(Mandatory = $true)][string]$Path)
  $values = @{}
  Get-Content -LiteralPath $Path -Encoding UTF8 | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) {
      return
    }
    $parts = $line.Split("=", 2)
    $value = $parts[1].Trim()
    if ($value.Length -ge 2 -and
        (($value.StartsWith('"') -and $value.EndsWith('"')) -or
          ($value.StartsWith("'") -and $value.EndsWith("'")))) {
      $value = $value.Substring(1, $value.Length - 2)
    }
    $values[$parts[0].Trim()] = $value
  }
  return $values
}

function Invoke-TestApi {
  param(
    [Parameter(Mandatory = $true)][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [AllowNull()][object]$Body = $null,
    [int]$TimeoutSeconds = 30
  )
  $request = @{
    Method = $Method
    Uri = "http://127.0.0.1:$Port$Path"
    Headers = $script:ApiHeaders
    TimeoutSec = $TimeoutSeconds
  }
  if ($null -ne $Body) {
    $request.ContentType = "application/json"
    $request.Body = $Body | ConvertTo-Json -Depth 100 -Compress
  }
  try {
    return Invoke-RestMethod @request
  } catch {
    $details = if ($_.ErrorDetails -and $_.ErrorDetails.Message) {
      $_.ErrorDetails.Message
    } else { $_.Exception.Message }
    throw "test API request failed: $details"
  }
}

function Wait-TestHealth {
  param([Parameter(Mandatory = $true)][System.Diagnostics.Process]$Process)
  $deadline = (Get-Date).AddSeconds(30)
  while ((Get-Date) -lt $deadline) {
    if ($Process.HasExited) { throw "multiturn server exited before health" }
    Start-Sleep -Milliseconds 250
    try {
      $health = Invoke-TestApi "Get" "/health" -TimeoutSeconds 2
      if ($health.ok) { return }
    } catch {
    }
  }
  throw "multiturn server did not become healthy"
}

function Wait-TestTurn {
  param(
    [Parameter(Mandatory = $true)][string]$ThreadId,
    [Parameter(Mandatory = $true)][string]$MessageId,
    [int]$TimeoutSeconds = 120
  )
  $deadline = (Get-Date).AddSeconds($TimeoutSeconds)
  while ((Get-Date) -lt $deadline) {
    Start-Sleep -Milliseconds 500
    $turn = Invoke-TestApi "Get" "/api/threads/$ThreadId/turn" -TimeoutSeconds 10
    if ($turn.userMessageId -ne $MessageId) { continue }
    if ($turn.status -in @("succeeded", "failed", "cancelled", "interrupted")) {
      return $turn
    }
  }
  throw "turn timed out: $MessageId"
}

function Get-TestEvents {
  param([Parameter(Mandatory = $true)][string]$ThreadId)
  $response = Invoke-TestApi "Get" "/api/threads/$ThreadId/events" -TimeoutSeconds 30
  if ($response -is [System.Array]) {
    return @($response)
  }
  if ($null -ne $response.PSObject.Properties["events"]) {
    return @($response.events)
  }
  return @($response)
}

function Get-TestMessages {
  param([Parameter(Mandatory = $true)][string]$ThreadId)
  $response = Invoke-TestApi "Get" "/api/threads/$ThreadId/messages" -TimeoutSeconds 30
  if ($response -is [System.Array]) {
    return @($response)
  }
  if ($null -ne $response.PSObject.Properties["messages"]) {
    return @($response.messages)
  }
  return @($response)
}

function Get-RequestStats {
  param([AllowNull()][object]$Event)
  if ($null -eq $Event) { return $null }
  $request = $Event.payload.request
  $json = $request | ConvertTo-Json -Depth 100
  return [ordered]@{
    seq = $Event.seq
    requestId = $Event.payload.request_id
    round = $Event.payload.round
    jsonCharacters = $json.Length
    roughJsonTokens = [Math]::Ceiling($json.Length / 4)
    conversationMessages = @($request.conversation).Count
    contextItems = @($request.contextItems).Count
    toolCandidates = @($request.toolCandidates).Count
    userMessageCharacters = ([string]$request.userMessage).Length
  }
}

function Convert-ToJsonBlock {
  param([AllowNull()][object]$Value)
  if ($null -eq $Value) { return "_not captured_" }
  $json = $Value | ConvertTo-Json -Depth 100
  return ('`````json' + "`n" + $json + "`n" + '`````')
}

$repoRoot = if ($PSScriptRoot) {
  Split-Path -Parent $PSScriptRoot
} else {
  [IO.Path]::GetFullPath((Get-Location).Path)
}
$values = Read-DotEnv $EnvFile
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) {
  throw "provider profile is incomplete"
}

$runId = "glm-multiturn-" + (Get-Date).ToUniversalTime().ToString("yyyyMMddTHHmmssZ")
$runRoot = Join-Path $repoRoot ".opentopia\evaluations\$runId"
$workspace = Join-Path $runRoot "workspace"
$database = Join-Path $runRoot "evaluation.db"
$sourceTrajectoryPath = Join-Path $runRoot "trajectory-multiturn.json"
$stdoutPath = Join-Path $runRoot "server.stdout.log"
$stderrPath = Join-Path $runRoot "server.stderr.log"
New-Item -ItemType Directory -Path $workspace -Force | Out-Null

$env:OPENTOPIA_API_KEY = $apiKey
$env:OPENTOPIA_OPENAI_BASE_URL = $baseUrl
$env:OPENTOPIA_MODEL = $model
$env:OPENTOPIA_API_TOKEN = "opentopia-multiturn-capture-token-0123456789abcdef0123456789abcdef"
$env:OPENTOPIA_CONTEXT_COMPACT_THRESHOLD_PERCENT = "95"
$env:OPENTOPIA_SANDBOX_MODE = "workspace-write"
$env:OPENTOPIA_SANDBOX_ENFORCEMENT = "best-effort"
$env:OPENTOPIA_SANDBOX_NETWORK = "deny"
$script:ApiHeaders = @{ Authorization = "Bearer $env:OPENTOPIA_API_TOKEN" }
$effectiveModel = if ($UseMockProvider) { "mock-control" } else { $model }

$serverPath = Join-Path $repoRoot ".opentopia\verify-target\debug\opentopia-server.exe"
if (-not (Test-Path -LiteralPath $serverPath -PathType Leaf)) {
  throw "server binary not found: $serverPath"
}
$server = Start-Process `
  -FilePath $serverPath `
  -ArgumentList @("--port", $Port, "--db", $database, "--permission", "full-access") `
  -RedirectStandardOutput $stdoutPath `
  -RedirectStandardError $stderrPath `
  -PassThru `
  -WindowStyle Hidden

try {
  Wait-TestHealth $server
  $settings = Invoke-TestApi "Get" "/api/settings"
  $activeProvider = @($settings.providers | Where-Object {
      $_.id -eq $settings.activeProviderId
    })[0]
  if (-not $activeProvider) { throw "active provider missing" }
  if ($UseMockProvider) {
    $activeProvider.kind = "mock"
    $activeProvider.model = "mock-control"
    $activeProvider.baseUrl = "http://127.0.0.1/mock"
  } else {
    $activeProvider.baseUrl = $baseUrl
    $activeProvider.model = $model
  }
  $activeProvider.contextWindowTokens = $ContextWindowTokens
  $activeProvider.maxOutputTokens = $MaxOutputTokens
  $activeProvider.reasoningEffort = "none"
  $activeProvider.supportsVision = $false
  Invoke-TestApi "Patch" "/api/settings" @{
    providers = @($settings.providers)
    activeProviderId = $settings.activeProviderId
  } | Out-Null

  $thread = Invoke-TestApi "Post" "/api/threads" @{
    title = "GLM multi-turn compaction capture"
    workspaceRoot = $workspace
  }
  $prompts = @(
    "多轮对话第 1 轮。请记住：项目代号是 Atlas，当前目标是设计一个可审计的上下文压缩方案。不要调用工具，只用一句中文确认。",
    "多轮对话第 2 轮。补充事实：压缩必须保留用户约束、未解决问题和下一步，不得把失败说成成功。不要调用工具，只用一句中文确认。",
    "多轮对话第 3 轮。补充事实：稳定的 system/developer 指令和工具 schema 应尽量保持不变。不要调用工具，只用一句中文确认。",
    "多轮对话第 4 轮。补充事实：历史对话应替换为结构化 checkpoint，并保留有界的最近对话尾部。不要调用工具，只用一句中文确认。",
    "多轮对话第 5 轮。补充事实：checkpoint 必须有 coverage、稳定 ID、source sequence 和 provider compatibility 信息。不要调用工具，只用一句中文确认。",
    "多轮对话第 6 轮。补充事实：压缩后要重新构造下一次模型请求，并记录压缩前后的请求内容。不要调用工具，只用一句中文确认。",
    "多轮对话第 7 轮。补充事实：任何自动摘要失败都必须保留失败原因，不能用人工结果冒充模型结果。不要调用工具，只用一句中文确认。",
    "多轮对话第 8 轮。请汇总目前已记住的 Atlas 方案事实，但仍只回复一句中文，不要调用工具。",
    "多轮对话第 9 轮。补充约束：压缩实验必须先完成多轮 user/assistant 对话，再触发压缩。只回复一句中文。",
    "多轮对话第 10 轮。最后确认：下一步是记录当前上下文快照、执行压缩、再捕获恢复请求。只回复一句中文。"
  )
  $turnDetail = (1..40 | ForEach-Object {
      "历史细节 $_：Atlas 方案要求把事实、约束、决策、未解决问题、来源序号和下一步分开保存，并在恢复时验证它们没有被静默删除。"
    }) -join " "
  $prompts = @($prompts | ForEach-Object { "$_`n`n$turnDetail" })
  $turnRecords = @()
  $completedCount = 0
  foreach ($prompt in $prompts | Select-Object -First $TurnCount) {
    $message = Invoke-TestApi "Post" "/api/threads/$($thread.id)/messages" @{
      content = $prompt
    }
    $turn = Wait-TestTurn $thread.id $message.id
    $turnRecords += [PSCustomObject]@{
      messageId = $message.id
      status = $turn.status
      assistantMessageCount = @($turn.assistantMessageIds).Count
    }
    if ($turn.status -eq "succeeded") {
      $completedCount += 1
    } else {
      break
    }
  }
  if ($completedCount -lt 4) {
    $failedTrajectory = Invoke-TestApi "Get" "/api/threads/$($thread.id)/trajectory" -TimeoutSeconds 30
    $failedTrajectoryPath = Join-Path $runRoot "trajectory-multiturn-failed.json"
    $failedTrajectoryJson = $failedTrajectory | ConvertTo-Json -Depth 100
    if ($failedTrajectoryJson.Contains($apiKey)) { throw "secret audit rejected failed trajectory" }
    [IO.File]::WriteAllText(
      $failedTrajectoryPath,
      "$failedTrajectoryJson`n",
      [Text.UTF8Encoding]::new($false)
    )
    $statusText = ($turnRecords | ForEach-Object { $_.status }) -join ","
    throw "fewer than four user/assistant turns completed: $completedCount; statuses=$statusText; trajectory=$failedTrajectoryPath"
  }

  $eventsBefore = @(Get-TestEvents $thread.id)
  $beforeMaxSeq = [int64](($eventsBefore | Measure-Object -Property seq -Maximum).Maximum)
  $messagesBefore = @(Get-TestMessages $thread.id)
  $summary = $null
  $compactError = $null
  $compactBody = @{}
  if ($UseMockProvider) {
    $sourceSeq = [int64]$beforeMaxSeq
    $compactBody = @{ checkpoint = @{
        goal = "Complete the Atlas context compaction experiment after a real multi-turn user/assistant conversation."
        userConstraints = @(
          @{ id = "atlas-facts-retained"; text = "Retain the Atlas project facts, constraints, decisions, unresolved issues, and next step from the preceding turns."; status = "active"; sourceSeqs = @($sourceSeq); confidence = 100 },
          @{ id = "no-false-success"; text = "Never claim an unfinished or failed compression experiment succeeded."; status = "active"; sourceSeqs = @($sourceSeq); confidence = 100 }
        )
        decisions = @(@{ id = "atlas-structured-checkpoint"; text = "Use a structured checkpoint and rebuild the next request with a bounded recent tail."; status = "active"; sourceSeqs = @($sourceSeq); confidence = 100 })
        workspaceState = @{ branch = $null; gitStatus = "No files were changed by the conversation turns."; filesChanged = @() }
        commandsAndValidation = @()
        openIssues = @()
        nextSteps = @(@{ id = "continue-atlas"; text = "Continue from the durable checkpoint and verify the recovered state."; status = "pending"; sourceSeqs = @($sourceSeq) })
        pendingInteractions = @()
        artifacts = @()
      } }
  }
  try {
    $summary = Invoke-TestApi "Post" "/api/threads/$($thread.id)/context/compact" $compactBody -TimeoutSeconds 120
  } catch {
    $compactError = $_.Exception.Message
  }
  $eventsAfterCompact = @(Get-TestEvents $thread.id)
  $compactionEvent = $eventsAfterCompact | Where-Object {
    $_.payload.type -eq "context_compacted" -and $_.seq -gt $beforeMaxSeq
  } | Select-Object -Last 1

  $recoveryPrompt = "请基于刚才的多轮 Atlas 对话继续。只用一句中文说明当前压缩后的任务状态，不要调用工具。"
  $recoveryMessage = Invoke-TestApi "Post" "/api/threads/$($thread.id)/messages" @{
    content = $recoveryPrompt
  }
  $afterRequest = $null
  $afterDeadline = (Get-Date).AddSeconds(90)
  while ((Get-Date) -lt $afterDeadline -and $null -eq $afterRequest) {
    Start-Sleep -Milliseconds 500
    $events = @(Get-TestEvents $thread.id)
    $afterRequest = $events | Where-Object {
      $_.payload.type -eq "model_request" -and
      $_.payload.round -gt 0 -and
      $null -ne $compactionEvent -and
      $_.seq -gt $compactionEvent.seq
    } | Select-Object -First 1
  }
  try {
    Invoke-TestApi "Post" "/api/threads/$($thread.id)/turn/cancel" @{} | Out-Null
  } catch {
  }

  $trajectory = Invoke-TestApi "Get" "/api/threads/$($thread.id)/trajectory" -TimeoutSeconds 30
  $trajectoryJson = $trajectory | ConvertTo-Json -Depth 100
  if ($trajectoryJson.Contains($apiKey)) { throw "secret audit rejected trajectory" }
  [IO.File]::WriteAllText($sourceTrajectoryPath, "$trajectoryJson`n", [Text.UTF8Encoding]::new($false))

  $events = @($trajectory.events)
  $compactionRequest = $events | Where-Object {
    $_.payload.type -eq "model_request" -and
    $_.payload.round -eq 0 -and
    $null -ne $compactionEvent -and
    $_.seq -gt $beforeMaxSeq -and
    $_.seq -lt $compactionEvent.seq
  } | Select-Object -Last 1
  $beforeRequest = $events | Where-Object {
    $_.payload.type -eq "model_request" -and
    $_.payload.round -gt 0 -and
    $_.seq -le $beforeMaxSeq
  } | Select-Object -Last 1
  if ($null -eq $beforeRequest) {
    $beforeRequest = $events | Where-Object {
      $_.payload.type -eq "model_request" -and $_.payload.round -gt 0
    } | Select-Object -Last 1
  }
  if ($null -eq $afterRequest -and $null -ne $compactionEvent) {
    $afterRequest = $events | Where-Object {
      $_.payload.type -eq "model_request" -and
      $_.payload.round -gt 0 -and
      $_.seq -gt $compactionEvent.seq
    } | Select-Object -First 1
  }
  $compactionResponse = if ($compactionRequest) {
    $events | Where-Object {
      $_.payload.type -eq "provider_response_received" -and
      $_.payload.request_id -eq $compactionRequest.payload.request_id
    } | Select-Object -Last 1
  } else { $null }
  $beforeStats = Get-RequestStats $beforeRequest
  $afterStats = Get-RequestStats $afterRequest
  $compactionStats = Get-RequestStats $compactionRequest
  $checkpoint = if ($compactionEvent) {
    $compactionEvent.payload.summary.checkpoint
  } else { $null }
  $metadata = if ($compactionEvent) {
    $compactionEvent.payload.summary.metadata
  } else { $null }
  $compactionUsage = if ($compactionResponse) {
    $compactionResponse.payload.body.usage
  } else { $null }
  $compactionRequestPayload = if ($compactionRequest) {
    $compactionRequest.payload.request
  } else { $null }
  $compactionResponsePayload = if ($compactionResponse) {
    $compactionResponse.payload.body
  } else { $null }
  $beforeRequestPayload = if ($beforeRequest) {
    $beforeRequest.payload.request
  } else { $null }
  $afterRequestPayload = if ($afterRequest) {
    $afterRequest.payload.request
  } else { $null }
  $compactionRequestJson = Convert-ToJsonBlock $compactionRequestPayload
  $compactionResponseJson = Convert-ToJsonBlock $compactionResponsePayload
  $beforeRequestJson = Convert-ToJsonBlock $beforeRequestPayload
  $afterRequestJson = Convert-ToJsonBlock $afterRequestPayload
  $status = if ($summary) { "succeeded" } else { "failed: $compactError" }
  $reportDate = (Get-Date).ToString("yyyy-MM-dd HH:mm:ss zzz")
  $report = @"
# GLM-5.2 真正多轮对话压缩实测

生成时间：$reportDate  
模型：$effectiveModel  
线程：$($thread.id)  
用户/助手已完成轮次：$completedCount / $($turnRecords.Count)  
压缩状态：$status  

## 1. 这次为什么是真正多轮

本次在同一线程连续发送 $($turnRecords.Count) 条独立 user message，每条都等待对应 assistant turn 结束。对话内容逐轮累积 Atlas 项目的事实、约束、决策和下一步，然后才读取快照并触发 context/compact。它不是把单次用户请求的工具调用 round 当成对话轮次。

$(if ($UseMockProvider) { "本次使用 OpenTopia mock provider 作为结构控制组：GLM relay 因 Desktop prompt/tool catalog 的视觉能力兼容错误无法完成第一轮，因此这里验证的是多轮历史重建和服务端压缩形态，不把 mock checkpoint 称为 GLM 摘要。" } else { "本次使用 GLM Provider。" })

## 2. 轮次结果

| 轮次 | message ID | 状态 |
| ---: | --- | --- |
$(($turnRecords | ForEach-Object { "| $([array]::IndexOf($turnRecords, $_) + 1) | $($_.messageId) | $($_.status) |" }) -join "`n")

压缩前事件 seq：$beforeMaxSeq  
压缩前 user message 数：$($messagesBefore.Count)  
压缩事件 seq：$(if ($compactionEvent) { $compactionEvent.seq } else { "未生成" })  
覆盖消息数：$(if ($checkpoint) { $checkpoint.coverage.throughMessageCount } else { "未生成" })

## 3. 压缩前后请求尺寸

| 项目 | 压缩前 | 压缩后 |
| --- | ---: | ---: |
| event seq | $(if ($beforeStats) { $beforeStats.seq } else { "-" }) | $(if ($afterStats) { $afterStats.seq } else { "-" }) |
| conversation messages | $(if ($beforeStats) { $beforeStats.conversationMessages } else { "-" }) | $(if ($afterStats) { $afterStats.conversationMessages } else { "-" }) |
| context items | $(if ($beforeStats) { $beforeStats.contextItems } else { "-" }) | $(if ($afterStats) { $afterStats.contextItems } else { "-" }) |
| 请求 JSON 字符数 | $(if ($beforeStats) { $beforeStats.jsonCharacters } else { "-" }) | $(if ($afterStats) { $afterStats.jsonCharacters } else { "-" }) |
| 粗略 JSON token | $(if ($beforeStats) { $beforeStats.roughJsonTokens } else { "-" }) | $(if ($afterStats) { $afterStats.roughJsonTokens } else { "-" }) |
| userMessage 字符数 | $(if ($beforeStats) { $beforeStats.userMessageCharacters } else { "-" }) | $(if ($afterStats) { $afterStats.userMessageCharacters } else { "-" }) |

## 4. 压缩模型请求与结果

自动压缩请求的完整脱敏 JSON：

$compactionRequestJson

Provider 返回的脱敏摘要：

$compactionResponseJson

## 5. 压缩输出 checkpoint

$(Convert-ToJsonBlock $checkpoint)

## 6. 压缩前模型请求（完整、已脱敏）

$beforeRequestJson

## 7. 压缩后第一次模型请求（完整、已脱敏）

$afterRequestJson

## 8. 原始证据

- 多轮轨迹：$($sourceTrajectoryPath.Replace($repoRoot + '\', ''))
- 压缩 request ID：$(if ($compactionRequest) { $compactionRequest.payload.request_id } else { "未捕获" })
- checkpoint ID：$(if ($checkpoint) { $checkpoint.id } else { "未生成" })
- 压缩前 request ID：$(if ($beforeRequest) { $beforeRequest.payload.request_id } else { "未捕获" })
- 压缩后 request ID：$(if ($afterRequest) { $afterRequest.payload.request_id } else { "未捕获" })
"@
  if ($report.Contains($apiKey)) { throw "secret audit rejected report" }
  $resolvedOutput = if ([IO.Path]::IsPathRooted($OutputPath)) {
    [IO.Path]::GetFullPath($OutputPath)
  } else {
    [IO.Path]::GetFullPath((Join-Path $repoRoot $OutputPath))
  }
  New-Item -ItemType Directory -Path (Split-Path -Parent $resolvedOutput) -Force | Out-Null
  [IO.File]::WriteAllText($resolvedOutput, "$report`n", [Text.UTF8Encoding]::new($false))
  [PSCustomObject]@{
    outputPath = $resolvedOutput
    trajectoryPath = $sourceTrajectoryPath
    threadId = $thread.id
    completedTurns = $completedCount
    compactionStatus = $status
    compactionSeq = if ($compactionEvent) { $compactionEvent.seq } else { $null }
    checkpointTokens = if ($metadata) { $metadata.checkpointTokens } else { $null }
    beforeRequestCharacters = if ($beforeStats) { $beforeStats.jsonCharacters } else { $null }
    afterRequestCharacters = if ($afterStats) { $afterStats.jsonCharacters } else { $null }
  } | ConvertTo-Json -Depth 10
} finally {
  if ($server -and -not $server.HasExited) {
    Stop-Process -Id $server.Id -Force
    $server.WaitForExit(5000) | Out-Null
  }
}
