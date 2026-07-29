param(
  [string]$TranslationReport = "docs/evaluations/context-compaction-long-horizon-2026-07-28.zh.md",
  [string]$OutputPath = "docs/evaluations/opentopia-prompt-readable-zh-2026-07-28.md"
)

$ErrorActionPreference = "Stop"

function Get-SectionJson {
  param([string]$Raw, [int]$SectionNumber)
  $sections = [regex]::Split($Raw, '(?m)^## ')
  $section = $sections | Where-Object { $_ -match "^$SectionNumber\. " } | Select-Object -First 1
  if (-not $section) { throw "Section $SectionNumber was not found" }
  $match = [regex]::Match($section, '(?ms)^`````json\s*(.*?)\s*^`````$')
  if (-not $match.Success) { throw "JSON block for section $SectionNumber was not found" }
  return ($match.Groups[1].Value | ConvertFrom-Json)
}

function Get-ContentText {
  param([object]$Item)
  $parts = New-Object System.Collections.Generic.List[string]
  foreach ($part in @($Item.content)) {
    if ($null -ne $part.text) {
      $parts.Add([string]$part.text)
    } elseif ($null -ne $part.value) {
      $parts.Add(($part.value | ConvertTo-Json -Depth 30 -Compress))
    } else {
      $parts.Add(($part | ConvertTo-Json -Depth 30 -Compress))
    }
  }
  return ($parts -join "`n")
}

function Add-TextBlock {
  param([System.Collections.Generic.List[string]]$Lines, [string]$Text)
  $Lines.Add('`````text')
  foreach ($line in ($Text -split "`r?`n")) { $Lines.Add($line) }
  $Lines.Add('`````')
  $Lines.Add('')
}

$reportPath = [IO.Path]::GetFullPath((Join-Path (Get-Location) $TranslationReport))
$outputPath = [IO.Path]::GetFullPath((Join-Path (Get-Location) $OutputPath))
$raw = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes($reportPath))
$request = Get-SectionJson $raw 4

$lines = New-Object System.Collections.Generic.List[string]
$lines.Add('# OpenTopia 实际模型提示词（中文可读视图）')
$lines.Add('')
$lines.Add('> 模型实际发送的 prompt 保持英文；本文件只把同一份请求解码并翻译成中文供人工检查。字段名、路径、ID、工具名和代码保持原样。')
$lines.Add('> 来源：`context-compaction-long-horizon-2026-07-28.zh.md` 第 4 节的压缩前 `model_request`。')
$lines.Add('')
$lines.Add('## 请求结构')
$lines.Add('')
$lines.Add("- context items：$(@($request.contextItems).Count)")
$lines.Add("- conversation messages：$(@($request.conversation).Count)")
$lines.Add("- tool candidates：$(@($request.toolCandidates).Count)")
$lines.Add('- 发送角色：system、developer、user；工具 schema 独立于正文。')
$lines.Add('')

$index = 0
foreach ($item in @($request.contextItems)) {
  $index++
  $role = if ($item.role) { [string]$item.role } else { 'unknown' }
  $kind = if ($item.kind) { [string]$item.kind } else { 'unknown' }
  $source = if ($item.source) { [string]$item.source } else { 'unknown' }
  $scope = if ($item.cacheScope) { [string]$item.cacheScope } else { 'unknown' }
  $lines.Add("## $index. $role / $kind")
  $lines.Add('')
  $lines.Add(("- source: ``{0}``" -f $source))
  $lines.Add(("- cache scope: ``{0}``" -f $scope))
  $lines.Add("- token estimate: $($item.tokenEstimate)")
  $lines.Add('')
  Add-TextBlock $lines (Get-ContentText $item)
}

if ($request.userMessage) {
  $lines.Add('## 当前 user message')
  $lines.Add('')
  Add-TextBlock $lines ([string]$request.userMessage)
}

$lines.Add('## 工具定义')
$lines.Add('')
$lines.Add('工具定义本身仍使用模型需要的原始 schema；中文可读视图只列出工具名，不把 schema 混入正文。')
$lines.Add('')
foreach ($tool in @($request.toolCandidates)) {
  $lines.Add(("- ``{0}``" -f $tool.name))
}
$lines.Add('')
$lines.Add('## 说明')
$lines.Add('')
$lines.Add('这份文件是可读视图，不是新的 provider payload。OpenTopia 仍向模型发送原始英文 prompt；压缩、哈希、角色和工具兼容性均使用原始结构。')

[IO.File]::WriteAllText($outputPath, ($lines -join "`n"), [Text.UTF8Encoding]::new($false))
Write-Output "Wrote $outputPath"
