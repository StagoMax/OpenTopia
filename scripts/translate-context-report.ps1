param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [Parameter(Mandatory = $true)][string]$TrajectoryPath,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$OutputPath = "docs\evaluations\context-prompt-zh-glm-2026-07-28.md"
)

$ErrorActionPreference = "Stop"

function Read-DotEnv {
  param([Parameter(Mandatory = $true)][string]$Path)
  $values = @{}
  Get-Content -LiteralPath $Path -Encoding UTF8 | ForEach-Object {
    $line = $_.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) { return }
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

$repoRoot = if ($PSScriptRoot) { Split-Path -Parent $PSScriptRoot } else { (Get-Location).Path }
$values = Read-DotEnv $EnvFile
$apiKey = [string]$values["${Profile}_API_KEY"]
$baseUrl = ([string]$values["${Profile}_BASE_URL"]).TrimEnd("/")
$model = [string]$values["${Profile}_MODEL"]
if (-not $apiKey -or -not $baseUrl -or -not $model) { throw "provider profile is incomplete" }

$trajectory = Get-Content -LiteralPath $TrajectoryPath -Raw -Encoding UTF8 | ConvertFrom-Json
$beforeRequest = $trajectory.events |
  Where-Object { $_.payload.type -eq "model_request" -and $_.payload.round -gt 0 } |
  Select-Object -First 1
if (-not $beforeRequest) { throw "no model request found in trajectory" }

$selected = @($beforeRequest.payload.request.contextItems | Where-Object {
    $_.kind -in @("base_instructions", "skill")
  })
if (-not $selected) { throw "base/skill prompt items not found" }
$source = ($selected | ForEach-Object {
    "===== $($_.kind) / $($_.id) =====`n$([string]$_.content[0].text)"
  }) -join "`n`n"

$translationPrompt = @"
把下面 OpenTopia 上下文提示词完整翻译成简体中文。

要求：
1. 保留原有标题、编号、列表、XML 标签、Markdown 代码围栏、路径、标识符、字段名和占位符。
2. 只翻译自然语言；不要翻译代码、JSON key、API 路径、文件名、模型名、协议名和工具名。
3. 不要总结，不要删节，不要解释，直接输出完整中文译文。

原文：
$source
"@

$body = @{
  model = $model
  messages = @(
    @{ role = "system"; content = "You are a precise technical translator. Output only the requested Simplified Chinese translation." }
    @{ role = "user"; content = $translationPrompt }
  )
  stream = $false
  temperature = 0.1
  max_tokens = 16384
  reasoning_effort = "none"
} | ConvertTo-Json -Depth 30

$headers = @{ Authorization = "Bearer $apiKey" }
try {
  $response = Invoke-RestMethod `
    -Method Post `
    -Uri "$baseUrl/chat/completions" `
    -Headers $headers `
    -ContentType "application/json" `
    -Body $body `
    -TimeoutSec 180
} catch {
  $details = if ($_.ErrorDetails -and $_.ErrorDetails.Message) { $_.ErrorDetails.Message } else { $_.Exception.Message }
  throw "GLM translation request failed: $details"
}

$translated = [string]$response.choices[0].message.content
if (-not $translated.Trim()) { throw "GLM translation response contained no text" }
if ($translated.Contains($apiKey)) { throw "secret audit rejected translation" }

$relativeSource = [IO.Path]::GetFullPath($TrajectoryPath).Replace($repoRoot + '\', '')
$report = @"
# OpenTopia 上下文提示词中文翻译

模型：$model  
来源轨迹：$relativeSource  
翻译范围：base_instructions 与 skill context item  

以下内容由 GLM Provider 直接翻译，保留原始结构，不是摘要：

$translated
"@
if ($report.Contains($apiKey)) { throw "secret audit rejected generated report" }
$resolvedOutput = if ([IO.Path]::IsPathRooted($OutputPath)) {
  [IO.Path]::GetFullPath($OutputPath)
} else {
  [IO.Path]::GetFullPath((Join-Path $repoRoot $OutputPath))
}
New-Item -ItemType Directory -Path (Split-Path -Parent $resolvedOutput) -Force | Out-Null
[IO.File]::WriteAllText($resolvedOutput, "$report`n", [Text.UTF8Encoding]::new($false))
[PSCustomObject]@{
  outputPath = $resolvedOutput
  model = $model
  sourceCharacters = $source.Length
  translatedCharacters = $translated.Length
} | ConvertTo-Json -Depth 10
