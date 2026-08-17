param(
  [string]$OriginalPath = "docs/evaluations/context-compaction-long-horizon-2026-07-28.md",
  [string]$TranslationPath = "docs/evaluations/context-compaction-long-horizon-2026-07-28.zh.md",
  [string]$EnvFile = "J:/Project/信贷审核助手/.env",
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$Model = "glm-5.2"
)

$ErrorActionPreference = "Stop"

function Read-DotEnv {
  param([string]$Path)
  $v = @{}
  foreach ($line0 in (Get-Content -LiteralPath $Path -Encoding UTF8)) {
    $line = $line0.Trim()
    if (-not $line -or $line.StartsWith("#") -or -not $line.Contains("=")) { continue }
    $p = $line.Split("=", 2); $x = $p[1].Trim()
    if ($x.Length -ge 2 -and (($x.StartsWith('"') -and $x.EndsWith('"')) -or ($x.StartsWith("'") -and $x.EndsWith("'")))) { $x = $x.Substring(1, $x.Length - 2) }
    $v[$p[0].Trim()] = $x
  }
  return $v
}

function Get-Leaves {
  param([object]$Value, [System.Collections.Generic.List[object]]$Items)
  if ($null -eq $Value) { return }
  if ($Value -is [string]) { $Items.Add($Value); return }
  if ($Value -is [System.Collections.IEnumerable] -and -not ($Value -is [string])) { foreach ($x in $Value) { Get-Leaves $x $Items }; return }
  foreach ($p in $Value.PSObject.Properties) { Get-Leaves $p.Value $Items }
}

function Set-Map {
  param([object]$Original, [object]$Target, [hashtable]$Map)
  if ($null -eq $Original -or $null -eq $Target) { return }
  if ($Original -is [string]) { return }
  if ($Original -is [System.Collections.IList] -and -not ($Original -is [string])) {
    for ($i = 0; $i -lt $Original.Count; $i++) {
      if ($Original[$i] -is [string]) { if ($Map.ContainsKey($Original[$i])) { $Target[$i] = $Map[$Original[$i]] } }
      else { Set-Map $Original[$i] $Target[$i] $Map }
    }
    return
  }
  foreach ($p in @($Original.PSObject.Properties)) {
    $q = $Target.PSObject.Properties[$p.Name]
    if ($null -eq $q) { continue }
    if ($p.Value -is [string]) { if ($Map.ContainsKey($p.Value)) { $q.Value = $Map[$p.Value] } }
    else { Set-Map $p.Value $q.Value $Map }
  }
}

function Invoke-Translate {
  param([System.Net.Http.HttpClient]$Client, [string]$Url, [string]$ModelName, [object[]]$Items)
  $instructions = 'Translate every items[i].text into Simplified Chinese. Return only valid JSON shaped as {"items":[{"id":integer,"text":"translation"}]}; preserve all ids and item count. Translate natural language only. Preserve code, commands, paths, filenames, identifiers, UUIDs, URLs, JSON keys, schema references, Markdown/XML tags, backticks, escape sequences, placeholders, and numbers exactly. Do not explain or summarize.'
  $payload = @{ model = $ModelName; thinking = @{ type = "disabled" }; temperature = 0.1; max_tokens = 5000; messages = @(@{ role = "system"; content = $instructions }, @{ role = "user"; content = ($Items | ConvertTo-Json -Depth 20 -Compress) }) }
  $body = $payload | ConvertTo-Json -Depth 30 -Compress
  $content = [System.Net.Http.StringContent]::new($body, [Text.Encoding]::UTF8, "application/json")
  $r = $Client.PostAsync($Url, $content).GetAwaiter().GetResult()
  $raw = $r.Content.ReadAsStringAsync().GetAwaiter().GetResult()
  if (-not $r.IsSuccessStatusCode) { throw "GLM returned HTTP $([int]$r.StatusCode)" }
  $answer = [string](($raw | ConvertFrom-Json).choices[0].message.content)
  if ($answer.StartsWith('```')) { $answer = [regex]::Replace($answer, '^```(?:json)?\s*', ''); $answer = [regex]::Replace($answer, '\s*```$', '') }
  $data = $answer.Trim() | ConvertFrom-Json
  $result = @{}
  foreach ($x in @($data.items)) { $result[[int]$x.id] = [string]$x.text }
  return $result
}

$original = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes((Join-Path (Get-Location) $OriginalPath)))
$translated = [Text.Encoding]::UTF8.GetString([IO.File]::ReadAllBytes((Join-Path (Get-Location) $TranslationPath)))
$os = [regex]::Split($original, '(?m)^## ')
$ts = [regex]::Split($translated, '(?m)^## ')
$originalObjects = @{}; $targetObjects = @{}; $unchanged = @{}
for ($n = 4; $n -le 7; $n++) {
  $om = [regex]::Match($os[$n], '(?ms)`````json\s*(.*?)\s*`````')
  $tm = [regex]::Match($ts[$n], '(?ms)`````json\s*(.*?)\s*`````')
  if (-not $om.Success -or -not $tm.Success) { throw "Missing JSON block in section $n" }
  $originalObjects[$n] = $om.Groups[1].Value | ConvertFrom-Json
  $targetObjects[$n] = $tm.Groups[1].Value | ConvertFrom-Json
  $ol = New-Object System.Collections.Generic.List[object]; $tl = New-Object System.Collections.Generic.List[object]
  Get-Leaves $originalObjects[$n] $ol; Get-Leaves $targetObjects[$n] $tl
  for ($i = 0; $i -lt [Math]::Min($ol.Count, $tl.Count); $i++) {
    $s = [string]$ol[$i]
    if ($s.Length -ge 3 -and $s -match '[A-Za-z]' -and $s -notmatch '^[A-Za-z0-9_./:#@?=+\-\\]+$' -and [string]$tl[$i] -eq $s) { $unchanged[$s] = $unchanged.Count }
  }
}
$items = @($unchanged.GetEnumerator() | ForEach-Object { [pscustomobject]@{ id = [int]$_.Value; text = [string]$_.Key } } | Sort-Object id)
Write-Output "Untranslated natural-language strings: $($items.Count)"

$values = Read-DotEnv $EnvFile; $key = [string]$values["${Profile}_API_KEY"]; $base = ([string]$values["${Profile}_BASE_URL"]).TrimEnd('/')
if (-not $key -or -not $base) { throw "Provider profile is incomplete" }
Add-Type -AssemblyName System.Net.Http
$client = [System.Net.Http.HttpClient]::new(); $client.Timeout = [TimeSpan]::FromMinutes(3); $client.DefaultRequestHeaders.Authorization = [System.Net.Http.Headers.AuthenticationHeaderValue]::new("Bearer", $key)
$map = @{}
try {
  $batch = New-Object System.Collections.Generic.List[object]; $chars = 0
  foreach ($item in $items) {
    if ($batch.Count -gt 0 -and ($chars + $item.text.Length) -gt 1800) {
      $got = Invoke-Translate $client ($base + '/chat/completions') $Model ([object[]]$batch.ToArray())
      foreach ($x in $batch) { if ($got.ContainsKey([int]$x.id)) { $map[$x.text] = $got[[int]$x.id] } else { $map[$x.text] = $x.text } }
      Write-Output "Translated $($batch.Count) strings"; $batch = New-Object System.Collections.Generic.List[object]; $chars = 0
    }
    $batch.Add($item); $chars += $item.text.Length
  }
  if ($batch.Count -gt 0) { $got = Invoke-Translate $client ($base + '/chat/completions') $Model ([object[]]$batch.ToArray()); foreach ($x in $batch) { if ($got.ContainsKey([int]$x.id)) { $map[$x.text] = $got[[int]$x.id] } else { $map[$x.text] = $x.text } }; Write-Output "Translated $($batch.Count) strings" }
} finally { $client.Dispose() }

for ($n = 4; $n -le 7; $n++) { Set-Map $originalObjects[$n] $targetObjects[$n] $map }
$out = New-Object System.Collections.Generic.List[string]; $out.Add($ts[0].TrimEnd());
for ($i = 1; $i -lt $ts.Length; $i++) {
  if ($i -ge 4 -and $i -le 7) {
    $out.Add("## " + (($ts[$i] -split "`n")[0].Trim())); $out.Add("")
    $tm = [regex]::Match($ts[$i], '(?ms)`````json\s*(.*?)\s*`````'); $before = $ts[$i].Substring(0, $tm.Index).Trim(); if ($before) { $out.Add($before); $out.Add("") }
    $out.Add('`````json'); $out.Add(($targetObjects[$i] | ConvertTo-Json -Depth 100)); $out.Add('`````'); $out.Add("")
  } else { $out.Add("## " + $ts[$i]) }
}
$targetFull = [IO.Path]::GetFullPath((Join-Path (Get-Location) $TranslationPath)); [IO.File]::WriteAllText($targetFull, ($out -join "`n"), [Text.UTF8Encoding]::new($false)); Write-Output "Wrote $targetFull"
