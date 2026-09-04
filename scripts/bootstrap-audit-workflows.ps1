[CmdletBinding()]
param(
  [string]$BaseUrl = "http://127.0.0.1:8787",
  [string]$ApiToken = $env:OPENTOPIA_API_TOKEN,
  [string]$WorkInjuryProject = "",
  [string]$CreditProject = "",
  [string]$Owner = "audit-platform-admin",
  [string]$Approver = "audit-risk-approver",
  [string]$ProviderConnectionId = "custom-provider-3",
  [string]$ModelId = "gpt-5.6-terra",
  [string]$ReasoningEffort = "medium",
  [switch]$SkipSeedEvents,
  [string]$TestQueuedCaseId = "",
  [switch]$ValidateDataOnly
)

$ErrorActionPreference = "Stop"
$repoRoot = [IO.Path]::GetFullPath((Split-Path -Parent $PSScriptRoot))
if (-not $WorkInjuryProject) {
  $WorkInjuryProject = Get-ChildItem -LiteralPath "J:\Project" -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName "data\knowledge_base\policy_chunks.json") } |
    Select-Object -First 1 -ExpandProperty FullName
}
if (-not $CreditProject) {
  $CreditProject = Get-ChildItem -LiteralPath "J:\Project" -Directory |
    Where-Object { Test-Path -LiteralPath (Join-Path $_.FullName ".venv\Scripts\credit-review-mcp.exe") } |
    Select-Object -First 1 -ExpandProperty FullName
}
if (-not $WorkInjuryProject -or -not $CreditProject) {
  throw "Audit project roots were not found; pass -WorkInjuryProject and -CreditProject."
}
$workInjuryRoot = [IO.Path]::GetFullPath($WorkInjuryProject)
$creditRoot = [IO.Path]::GetFullPath($CreditProject)
$headers = @{}
if ($ApiToken) { $headers.Authorization = "Bearer $ApiToken" }

$workInjuryNamespace = "opentopia.audit.work-injury.v2"
$creditNamespace = "opentopia.audit.credit-review.v1"

function Invoke-TopiaApi {
  param(
    [Parameter(Mandatory = $true)][ValidateSet("GET", "POST", "PUT", "PATCH")][string]$Method,
    [Parameter(Mandatory = $true)][string]$Path,
    [object]$Body = $null,
    [int]$TimeoutSec = 300
  )
  $parameters = @{
    Method = $Method
    Uri = "$($BaseUrl.TrimEnd('/'))$Path"
    Headers = $headers
    TimeoutSec = $TimeoutSec
  }
  if ($null -ne $Body) {
    $parameters.ContentType = "application/json; charset=utf-8"
    $parameters.Body = $Body | ConvertTo-Json -Depth 100 -Compress
  }
  try {
    Invoke-RestMethod @parameters
  }
  catch {
    $detail = [string]$_.ErrorDetails.Message
    throw "$Method $Path failed: $($_.Exception.Message) $detail"
  }
}

function Expand-TopiaItems {
  param([object]$Value)
  if ($null -eq $Value) { return @() }
  if ($Value.PSObject.Properties.Name -contains "items") { return @($Value.items) }
  return @($Value)
}

function Wait-FlowRunStatus {
  param([string]$RunId, [string[]]$Statuses, [int]$TimeoutSeconds = 300)
  $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
  do {
    $run = Invoke-TopiaApi GET "/api/flow-runs/$RunId"
    if ($run.status -in $Statuses) { return $run }
    Start-Sleep -Milliseconds 250
  } while ([DateTime]::UtcNow -lt $deadline)
  throw "Flow Run $RunId did not reach $($Statuses -join ', ')"
}

function Resolve-FlowReviewTask {
  param([object]$Run, [string]$TaskType, [string]$IdempotencyKey)
  $task = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($Run.id)")) |
    Where-Object { $_.taskType -eq $TaskType } |
    Select-Object -First 1
  if (-not $task) { throw "Flow Run $($Run.id) is missing $TaskType HumanTask" }
  Invoke-TopiaApi POST "/api/human-tasks/$($task.id)/resolve" @{
    expectedRevision = $task.revision
    action = "approve"
    idempotencyKey = $IdempotencyKey
  } | Out-Null
}

function Import-SagText {
  param(
    [string]$Namespace,
    [string]$SourceKey,
    [string]$Filename,
    [string]$Title,
    [string]$Content,
    [hashtable]$Metadata
  )
  Invoke-TopiaApi POST "/api/library/sag/ingestions/text" @{
    namespace = $Namespace
    sourceKey = $SourceKey
    filename = $Filename
    title = $Title
    content = $Content
    metadata = $Metadata
  } | Out-Null
}

function Get-StableSourceId {
  param([Parameter(Mandatory = $true)][string]$Value)
  $sha = [Security.Cryptography.SHA256]::Create()
  try {
    $bytes = [Text.Encoding]::UTF8.GetBytes($Value.Trim())
    $hash = [BitConverter]::ToString($sha.ComputeHash($bytes)).Replace("-", "").ToLowerInvariant()
    return $hash.Substring(0, 16)
  }
  finally {
    $sha.Dispose()
  }
}

function Get-WorkInjuryKnowledgeSources {
  $corpusFiles = @("policy_chunks.json", "generated_policy_chunks.json")
  $records = foreach ($corpusFile in $corpusFiles) {
    $path = Join-Path $workInjuryRoot "data\knowledge_base\$corpusFile"
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
      throw "Work injury knowledge file not found: $path"
    }
    $chunks = @(Get-Content -LiteralPath $path -Raw -Encoding UTF8 | ConvertFrom-Json)
    foreach ($chunk in $chunks) {
      if (-not ([string]$chunk.source).Trim()) {
        throw "Work injury chunk '$($chunk.chunk_id)' is missing its logical source"
      }
      [pscustomobject]@{ CorpusFile = $corpusFile; Chunk = $chunk }
    }
  }

  $sourceGroups = @($records | Group-Object { [string]$_.Chunk.source } | Sort-Object Name)
  foreach ($sourceGroup in $sourceGroups) {
    $sourceTitle = [string]$sourceGroup.Name
    $sourceId = Get-StableSourceId $sourceTitle
    $sourceRecords = @($sourceGroup.Group)
    $sourceUrls = @($sourceRecords | ForEach-Object { [string]$_.Chunk.source_url } | Where-Object { $_ } | Sort-Object -Unique)
    $categories = @($sourceRecords | ForEach-Object { [string]$_.Chunk.category } | Where-Object { $_ } | Sort-Object -Unique)
    $jurisdictions = @($sourceRecords | ForEach-Object { [string]$_.Chunk.jurisdiction } | Where-Object { $_ } | Sort-Object -Unique)
    $corpora = @($sourceRecords | ForEach-Object { $_.CorpusFile } | Sort-Object -Unique)
    $sections = foreach ($record in $sourceRecords) {
      $chunk = $record.Chunk
      $tags = @($chunk.tags) -join ", "
      @(
        "## $($chunk.title)",
        "- Chunk ID: $($chunk.chunk_id)",
        "- Source URL: $($chunk.source_url)",
        "- Jurisdiction: $($chunk.jurisdiction)",
        "- Category: $($chunk.category)",
        "- Risk tags: $tags",
        "",
        [string]$chunk.text,
        ""
      ) -join "`n"
    }
    $content = "# $sourceTitle`n`n" + ($sections -join "`n")
    [ordered]@{
      sourceKey = "opentopia/audit/work-injury/source/$sourceId"
      filename = "work-injury-source-$sourceId.md"
      title = $sourceTitle
      content = $content
      sourceChunkCount = $sourceRecords.Count
      metadata = @{
        domain = "work_injury_audit"
        originProject = $workInjuryRoot
        sourceTitle = $sourceTitle
        sourceUrls = $sourceUrls
        categories = $categories
        jurisdictions = $jurisdictions
        corpusFiles = $corpora
        sourceChunkCount = $sourceRecords.Count
        sourceType = $(if ($sourceUrls.Count -gt 0) { "public_reference" } else { "derived_or_user_provided_reference" })
        importedBy = "bootstrap-audit-workflows"
      }
    }
  }
}

function Import-WorkInjuryKnowledge {
  $sources = @(Get-WorkInjuryKnowledgeSources)
  foreach ($source in $sources) {
    Import-SagText `
      -Namespace $workInjuryNamespace `
      -SourceKey $source.sourceKey `
      -Filename $source.filename `
      -Title $source.title `
      -Content $source.content `
      -Metadata $source.metadata
  }
  [ordered]@{
    namespace = $workInjuryNamespace
    sourceCount = $sources.Count
    sourceChunkCount = ($sources | ForEach-Object { [int]$_.sourceChunkCount } | Measure-Object -Sum).Sum
  }
}

function Import-CreditKnowledge {
  $knowledgeRoot = Join-Path $creditRoot "data\knowledge_sources"
  $files = @(Get-ChildItem -LiteralPath $knowledgeRoot -Recurse -File -Filter *.md | Sort-Object FullName)
  if ($files.Count -eq 0) { throw "No credit knowledge Markdown files found under $knowledgeRoot" }
  foreach ($file in $files) {
    $relative = $file.FullName.Substring($knowledgeRoot.Length).TrimStart("\", "/").Replace("\", "/")
    $synthetic = $relative.StartsWith("synthetic_sop/", [StringComparison]::OrdinalIgnoreCase)
    Import-SagText `
      -Namespace $creditNamespace `
      -SourceKey "opentopia/audit/credit-review/$relative" `
      -Filename $file.Name `
      -Title $file.BaseName.Replace("_", " ") `
      -Content (Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8) `
      -Metadata @{
        domain = "credit_review"
        originProject = $creditRoot
        relativePath = $relative
        sourceType = $(if ($synthetic) { "synthetic_internal_sop" } else { "public_reference" })
        synthetic = $synthetic
        importedBy = "bootstrap-audit-workflows"
      }
  }
  [ordered]@{
    namespace = $creditNamespace
    sourceCount = $files.Count
    chunking = "managed_by_sag"
  }
}

function Get-OrCreateMcpServer {
  param(
    [string]$Name,
    [string[]]$LegacyNames = @(),
    [string]$Command,
    [string[]]$Arguments,
    [string]$WorkingDirectory
  )
  $knownNames = @($Name) + @($LegacyNames)
  $existing = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/mcp/servers")) |
    Where-Object { $_.server.name -in $knownNames } |
    Select-Object -First 1
  if ($existing) {
    return Invoke-TopiaApi PATCH "/api/mcp/servers/$($existing.server.serverId)" @{
      name = $Name
      command = $Command
      args = $Arguments
      cwd = $WorkingDirectory
      envKeys = @()
      timeoutMs = 120000
      enabled = $true
    }
  }
  Invoke-TopiaApi POST "/api/mcp/servers" @{
    name = $Name
    command = $Command
    args = $Arguments
    cwd = $WorkingDirectory
    envKeys = @()
    timeoutMs = 120000
    enabled = $true
  }
}

function Get-ConnectionAccess {
  param([object]$McpServer, [string[]]$ToolNames)
  $connectionId = [string]$McpServer.server.serverId
  $refresh = Invoke-TopiaApi POST "/api/connections/$connectionId/capabilities/refresh" $null 120
  $capabilities = @($refresh.capabilityRevision.capabilities)
  $operationIds = foreach ($toolName in $ToolNames) {
    $capability = $capabilities |
      Where-Object { $_.providerMetadata.toolName -eq $toolName } |
      Select-Object -First 1
    if (-not $capability) {
      throw "MCP tool '$toolName' was not discovered for Connection $connectionId"
    }
    [string]$capability.capabilityId
  }
  @{
    ConnectionId = $connectionId
    CapabilityRevision = [int]$refresh.capabilityRevision.revision
    OperationIds = @($operationIds)
  }
}

function ConvertTo-PythonStringLiteral {
  param([string]$Value)
  '"' + $Value.Replace('\', '\\').Replace('"', '\"') + '"'
}

function Get-VenvBasePython {
  param([string]$VenvPython)
  $basePython = (& $VenvPython -c "import sys; print(sys._base_executable)" 2>&1 | Select-Object -Last 1).ToString().Trim()
  if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $basePython -PathType Leaf)) {
    throw "Unable to resolve the base Python runtime for $VenvPython"
  }
  $basePython
}

function New-AgentTemplate {
  param(
    [string]$TemplateId,
    [string]$Name,
    [string]$Instructions,
    [string[]]$Tools = @(),
    [hashtable]$ConnectionAccess = $null,
    [string]$KnowledgeNamespace = ""
  )
  $toolSet = @($Tools | Sort-Object -Unique)
  if ($KnowledgeNamespace -and "library_search" -notin $toolSet) {
    $toolSet += "library_search"
  }
  $spec = @{
    description = "OpenTopia 内置审计工作流智能体模板：$Name"
    instructions = $Instructions
    capabilities = @{
      allowAllTools = $false; tools = $toolSet
      allowAllSkills = $false; skills = @()
      allowAllPlugins = $false; plugins = @()
      allowAllMcpServers = $false; mcpServers = @()
      allowAllWorkspaceRoots = $false; workspaceRoots = @($repoRoot)
    }
    connectionBindings = @()
    resourceGrants = @()
    modelPolicy = @{ allowAllModels = $true; allowedModels = @() }
    stateSchema = @{ type = "object" }
    outputSchema = @{ type = "object" }
    allowAllDelegates = $false
    delegateTemplateIds = @()
    budget = @{ maxTurns = 12; maxToolCalls = 20; maxDurationSeconds = 600 }
    riskClass = "high"
  }
  if ($ConnectionAccess) {
    $spec.connectionBindings = @(@{
      connectionId = $ConnectionAccess.ConnectionId
      capabilityRevision = $ConnectionAccess.CapabilityRevision
      operationGrants = @($ConnectionAccess.OperationIds | ForEach-Object { @{ operationId = $_ } })
    })
  }
  if ($KnowledgeNamespace) {
    $spec.knowledgeBinding = @{ namespaces = @($KnowledgeNamespace) }
  }
  $draft = Invoke-TopiaApi POST "/api/agent-templates" @{
    templateId = $TemplateId
    name = $Name
    owner = $Owner
    spec = $spec
  }
  (Invoke-TopiaApi POST "/api/agent-templates/$TemplateId/versions/$($draft.template.version)/publish" @{
    approvedBy = $Owner
    approveCapabilityExpansion = $true
  }).template
}

function Get-WorkInjuryDemoEvents {
  param([Parameter(Mandatory = $true)][string]$ConnectionId)
  $caseRoot = Join-Path $workInjuryRoot "data\synthetic_cases"
  $files = @(Get-ChildItem -LiteralPath $caseRoot -File -Filter *.json | Sort-Object Name)
  if ($files.Count -eq 0) { throw "No work injury demo cases found under $caseRoot" }
  foreach ($file in $files) {
    $case = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    $caseId = ([string]$case.case_id).Trim()
    if (-not $caseId) { throw "Work injury demo case is missing case_id: $($file.FullName)" }
    $expenseLines = @($case.expense_lines)
    $expenseTotal = ($expenseLines | Measure-Object -Property amount -Sum).Sum
    [ordered]@{
      idempotencyKey = "demo-event:work-injury:${caseId}:node-trigger-v2"
      caseId = $caseId
      payload = [ordered]@{
        caseId = $caseId
        eventKind = "work_injury_medical_expense_review"
        payloadRef = "connection://$ConnectionId/demo-cases/$([Uri]::EscapeDataString($caseId))"
        synthetic = $true
        summary = [ordered]@{
          region = [string]$case.region
          accidentDate = [string]$case.accident_date
          expenseTotal = [Math]::Round([double]$expenseTotal, 2)
          expenseCount = $expenseLines.Count
          materialCount = @($case.materials).Count
          recognizedDiagnosisCount = @($case.recognized_diagnoses).Count
        }
      }
    }
  }
}

function Get-CreditDemoCaseTitle {
  param([Parameter(Mandatory = $true)][string]$CaseId)

  $match = [regex]::Match($CaseId, '^case_(?:(?<number>\d+)_)?(?<scenario>.+)$')
  if (-not $match.Success) { throw "Unsupported credit review demo case ID: $CaseId" }

  $scenarioLabel = switch ($match.Groups["scenario"].Value) {
    "normal" { "正常材料" }
    "normal_salary" { "正常工资收入" }
    "income_mismatch" { "收入不一致" }
    "cashflow_anomaly" { "现金流异常" }
    "credit_pressure" { "信贷压力" }
    "purpose_mismatch" { "贷款用途不一致" }
    "combo" { "组合风险" }
    "invoice_qr_mismatch" { "发票二维码不一致" }
    "verified_but_logic_suspicious" { "核验通过但逻辑可疑" }
    default { throw "Unsupported credit review demo scenario: $($match.Groups['scenario'].Value)" }
  }
  $caseNumber = $match.Groups["number"].Value
  $caseLabel = if ($caseNumber) { "信贷案件 $([int]$caseNumber)" } else { "信贷案件" }
  "$caseLabel · $scenarioLabel"
}

function Get-CreditDemoEvents {
  param([Parameter(Mandatory = $true)][string]$ConnectionId)
  $caseRoot = Join-Path $creditRoot "data\synthetic_cases"
  $files = @(Get-ChildItem -LiteralPath $caseRoot -File -Filter *.json | Sort-Object Name)
  if ($files.Count -eq 0) { throw "No credit review demo cases found under $caseRoot" }
  foreach ($file in $files) {
    $case = Get-Content -LiteralPath $file.FullName -Raw -Encoding UTF8 | ConvertFrom-Json
    $caseId = ([string]$case.case_id).Trim()
    if (-not $caseId) { throw "Credit review demo case is missing case_id: $($file.FullName)" }
    $documents = @($case.documents)
    $application = @($documents | Where-Object { $_.document_type -eq "loan_application" } | Select-Object -First 1)
    $applicationFields = if ($application.Count -gt 0) { $application[0].fields } else { $null }
    [ordered]@{
      idempotencyKey = "demo-event:credit-review:${caseId}:node-trigger-v3"
      caseId = $caseId
      payload = [ordered]@{
        title = Get-CreditDemoCaseTitle -CaseId $caseId
        caseId = $caseId
        eventKind = "credit_material_review"
        payloadRef = "connection://$ConnectionId/demo-cases/$([Uri]::EscapeDataString($caseId))"
        synthetic = $true
        summary = [ordered]@{
          loanAmount = if ($null -ne $applicationFields) { $applicationFields.loan_amount } else { $null }
          loanTermMonths = if ($null -ne $applicationFields) { $applicationFields.loan_term_months } else { $null }
          loanPurpose = if ($null -ne $applicationFields) { [string]$applicationFields.loan_purpose } else { "" }
          documentCount = $documents.Count
          documentTypes = @($documents | ForEach-Object { [string]$_.document_type } | Sort-Object -Unique)
        }
      }
    }
  }
}

function Seed-DemoEvents {
  param(
    [Parameter(Mandatory = $true)][object]$Flow,
    [Parameter(Mandatory = $true)][object[]]$Events
  )
  $caseIds = @($Events | ForEach-Object { [string]$_.caseId })
  if (($caseIds | Sort-Object -Unique).Count -ne $caseIds.Count) {
    throw "Demo event case IDs must be unique for Flow $($Flow.flowId)"
  }
  $idempotencyKeys = @($Events | ForEach-Object { [string]$_.idempotencyKey })
  if (($idempotencyKeys | Sort-Object -Unique).Count -ne $idempotencyKeys.Count) {
    throw "Demo event idempotency keys must be unique for Flow $($Flow.flowId)"
  }
  $seeded = foreach ($event in $Events) {
    $result = Invoke-TopiaApi POST "/api/flows/$([Uri]::EscapeDataString($Flow.flowId))/invoke" @{
      idempotencyKey = [string]$event.idempotencyKey
      input = $event.payload
    }
    if ($result.case.flowId -ne $Flow.flowId -or $result.case.flowRevisionId -ne $Flow.activeRevision.id) {
      throw "Demo event '$($event.caseId)' was routed outside its target Flow Revision"
    }
    if ($result.case.status -ne "accepted" -or $result.case.flowRunId -or $result.run) {
      throw "Demo event '$($event.caseId)' did not remain pending for human trigger review"
    }
    [ordered]@{
      caseId = [string]$event.caseId
      caseIdRecord = [string]$result.case.id
      reused = [bool]$result.reused
    }
  }
  [ordered]@{
    requested = $Events.Count
    accepted = @($seeded).Count
    events = @($seeded)
  }
}

function Test-QueuedAuditCase {
  param(
    [Parameter(Mandatory = $true)][object[]]$Flows,
    [Parameter(Mandatory = $true)][string]$CaseId
  )
  $matches = @(
    foreach ($flow in $Flows) {
      foreach ($event in @($flow.demoEventBatch.events)) {
        if ([string]$event.caseId -eq $CaseId) {
          [ordered]@{ flow = $flow; event = $event }
        }
      }
    }
  )
  if ($matches.Count -ne 1) {
    throw "Expected exactly one queued Demo event for case '$CaseId', found $($matches.Count)"
  }
  $match = $matches[0]
  $started = Invoke-TopiaApi POST "/api/flow-cases/$($match.event.caseIdRecord)/start" @{}
  if ($started.case.status -ne "started" -or -not $started.run.id) {
    throw "Queued Demo event '$CaseId' did not start"
  }
  $run = Wait-FlowRunStatus $started.run.id @("waiting_approval", "waiting_human", "succeeded", "failed", "cancelled")
  if ($run.status -eq "waiting_approval") {
    Resolve-FlowReviewTask $run "approval" "migration-case-approval-$($run.id)"
    $run = Wait-FlowRunStatus $run.id @("waiting_human", "succeeded", "failed", "cancelled")
  }
  if ($run.status -eq "waiting_human") {
    Resolve-FlowReviewTask $run "output_review" "migration-case-output-$($run.id)"
    $run = Wait-FlowRunStatus $run.id @("succeeded", "failed", "cancelled")
  }
  if ($run.status -ne "succeeded") {
    throw "Queued Demo event '$CaseId' failed: $($run.status) $($run.error)"
  }
  $agentNodes = @($run.nodeRuns | Where-Object { $_.nodeId -in @("domain_audit", "sag_evidence", "review_report") })
  if ($agentNodes.Count -ne 3 -or @($agentNodes | Where-Object { $_.status -ne "succeeded" }).Count -gt 0) {
    throw "Queued Demo event '$CaseId' did not complete all three Agent nodes"
  }
  [ordered]@{
    caseId = $CaseId
    flowCaseId = [string]$started.case.id
    flowRunId = [string]$run.id
    flowRevisionId = [string]$run.flowRevisionId
    status = [string]$run.status
    agentNodes = @($agentNodes | ForEach-Object { [ordered]@{ nodeId = [string]$_.nodeId; status = [string]$_.status; toolCalls = [int]$_.toolCalls } })
    output = $run.output
  }
}

function New-AuditFlow {
  param(
    [string]$FlowId,
    [string]$Name,
    [object]$DomainTemplate,
    [object]$EvidenceTemplate,
    [object]$ReportTemplate,
    [string]$EventSource,
    [string]$EventType,
    [string]$TestCaseId,
    [object]$InputSummarySchema,
    [object]$OutputSchema,
    [object[]]$DemoEvents
  )
  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "$Name - 人工确认事件队列"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }
  $thread = Invoke-TopiaApi PUT "/api/threads/$($thread.id)/model" @{
    selection = @{
      connectionId = $ProviderConnectionId
      modelId = $ModelId
      reasoningEffort = $ReasoningEffort
    }
  }
  if ($thread.modelSelection.connectionId -ne $ProviderConnectionId -or $thread.modelSelection.modelId -ne $ModelId) {
    throw "Flow thread was not pinned to $ProviderConnectionId / $ModelId"
  }
  $inputSchema = @{
    type = "object"
    required = @("caseId")
    properties = @{
      title = @{ type = "string"; title = "案件名称" }
      caseId = @{ type = "string"; title = "案件 ID" }
      eventKind = @{ type = "string"; title = "事件类型" }
      payloadRef = @{ type = "string"; title = "载荷引用" }
      synthetic = @{ type = "boolean"; title = "合成数据" }
      summary = $InputSummarySchema
    }
    additionalProperties = $false
  }
  $objectSchema = @{ type = "object" }
  # The ingress Trigger belongs to the entry Agent and is frozen into the Flow
  # definition before validation, Test Run, or Flow activation.
  $triggerId = [guid]::NewGuid().ToString()
  $eventActivation = @{
    expression = @{
      operator = "source"
      source = @{
        kind = "event_subscription"
        triggerId = $triggerId
        source = $EventSource
        eventType = $EventType
      }
    }
    ingressPolicy = "require_review"
  }
  $domainFinalActivation = @{
    expression = @{
      operator = "source"
      source = @{ kind = "agent_final"; nodeId = "domain_audit" }
    }
    ingressPolicy = "immediate"
  }
  $node = {
    param($Id, $Label, $Template, $Instructions, $Activation = $null)
    $config = @{
      reference = $Template.templateId
      templateVersion = $Template.version
      instructions = $Instructions
    }
    if ($null -ne $Activation) { $config.activation = $Activation }
    $nodeOutputSchema = if ($Id -eq "review_report") { $OutputSchema } else { $objectSchema }
    @{
      id = $Id
      label = $Label
      kind = "agent"
      config = $config
      inputSchema = $objectSchema
      outputSchema = $nodeOutputSchema
    }
  }
  $nodes = @(
    (& $node "domain_audit" "结构化审核" $DomainTemplate "将 @Flow.input 作为不可变的案件事件，将 @Trigger.input 作为本智能体的触发载荷。调用已批准的外部 SAG 领域审核工具，不得调用旧版 RAG 工具。保留 caseId、runId、规则发现、依据 ID 和政策检索词；所有面向审核人员的说明使用简体中文。" $eventActivation),
    (& $node "sag_evidence" "SAG 政策依据" $EvidenceTemplate "本智能体由 domain_audit 的 Agent Final 通知触发。处理 @Trigger.input 中的 domain_audit 产物，同时保留 @Flow.input 中的原始案件。对每个政策检索词调用 library_search；只使用模板冻结的命名空间，并返回标题、sourcePath、eventId/evidenceId 和简体中文摘要。" $domainFinalActivation),
    @{ id = "evidence_validator"; label = "依据完整性检查"; kind = "validator"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "review_gate"; label = "人工复核关口"; kind = "approval"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "review_context"; label = "汇总复核上下文"; kind = "join"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    (& $node "review_report" "复核报告" $ReportTemplate "将结构化事实、规则发现和 SAG 引用合并为简体中文 JSON 复核草案。本流程只辅助人工复核，不得自动作出批准、拒绝、支付、授信或其他待遇决定。"),
    @{ id = "output"; label = "输出待人工复核"; kind = "output"; config = @{}; inputSchema = $OutputSchema; outputSchema = $OutputSchema }
  )
  $edges = @(
    @{ from = "domain_audit"; to = "sag_evidence"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "sag_evidence"; to = "evidence_validator"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "evidence_validator"; to = "review_gate"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "domain_audit"; to = "review_context"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "sag_evidence"; to = "review_context"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "evidence_validator"; to = "review_context"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "review_gate"; to = "review_context"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "review_context"; to = "review_report"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "review_report"; to = "output"; allowedFields = @(); dataClassification = "confidential" }
  )
  $spec = @{
    flowId = $FlowId
    name = $Name
    description = "事件先进入待处理队列；人工确认启动后，依次执行结构化审核、隔离的 SAG 依据检索和复核报告起草。"
    owner = $Owner
    categories = @("audit", "human-in-the-loop", "sag")
    source = @{ kind = "natural_language"; description = "智能体 + 隔离 SAG + 人工确认事件入口" }
    inputSchema = $inputSchema
    outputSchema = $OutputSchema
    graph = @{ schemaVersion = 1; entryNodeId = "domain_audit"; nodes = $nodes; edges = $edges }
    requestedCapabilities = @{}
    budget = @{ maxNodeExecutions = 8; maxToolCalls = 30; maxDurationSeconds = 1200; maxLoopIterations = 1 }
    riskClass = "high"
    pendingDecisions = @()
  }
  $draft = Invoke-TopiaApi POST "/api/threads/$($thread.id)/flow-drafts" @{ spec = $spec }
  $validated = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/validate"
  if (-not $validated.draft.lastValidation.valid) {
    throw "Flow validation failed: $($validated.draft.lastValidation.issues | ConvertTo-Json -Depth 20 -Compress)"
  }
  $trial = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/simulate" @{ input = @{ caseId = $TestCaseId } }
  if ($trial.status -ne "passed") { throw "Flow simulation failed for $FlowId" }
  $testRun = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/test-run" @{
    input = @{ caseId = $TestCaseId }
    startedBy = $Owner
  }
  $testRun = Wait-FlowRunStatus $testRun.id @("waiting_approval", "waiting_human", "succeeded", "failed", "cancelled")
  if ($testRun.status -eq "waiting_approval") {
    Resolve-FlowReviewTask $testRun "approval" "bootstrap-test-approval-$($testRun.id)"
    $testRun = Wait-FlowRunStatus $testRun.id @("waiting_human", "succeeded", "failed", "cancelled")
  }
  if ($testRun.status -eq "waiting_human") {
    Resolve-FlowReviewTask $testRun "output_review" "bootstrap-test-output-$($testRun.id)"
    $testRun = Wait-FlowRunStatus $testRun.id @("succeeded", "failed", "cancelled")
  }
  if ($testRun.status -ne "succeeded") {
    throw "Flow Test Run failed for ${FlowId}: $($testRun.status) $($testRun.error)"
  }
  $existing = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/flows?query=$([Uri]::EscapeDataString($FlowId))")) |
    Where-Object { $_.flowId -eq $FlowId } |
    Select-Object -First 1
  $activation = @{
    activatedBy = $Approver
    outputReviewPolicy = "always_review_output"
    output = @{ kind = "inbox" }
  }
  if ($existing) { $activation.expectedFlowRevision = [int]$existing.revision }
  $flow = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/activate" $activation
  if ($flow.activeRevision.trigger.triggerId -ne $triggerId -or $flow.activeRevision.ingressPolicy -ne "require_review") {
    throw "Flow did not inherit the entry Agent Trigger and review policy"
  }
  $seed = [ordered]@{ requested = 0; accepted = 0; events = @() }
  if (-not $SkipSeedEvents) {
    $seed = Seed-DemoEvents -Flow $flow -Events $DemoEvents
  }
  @{
    threadId = $thread.id
    flowId = $flow.flowId
    flowRevisionId = $flow.activeRevision.id
    flowVersion = $flow.activeRevision.compiledWorkflow.flowVersion
    ingressPolicy = $flow.activeRevision.ingressPolicy
    demoEventBatch = $seed
  }
}

if ($ValidateDataOnly) {
  $workSources = @(Get-WorkInjuryKnowledgeSources)
  $creditSourceRoot = Join-Path $creditRoot "data\knowledge_sources"
  $creditSources = @(Get-ChildItem -LiteralPath $creditSourceRoot -Recurse -File -Filter *.md)
  $workEvents = @(Get-WorkInjuryDemoEvents -ConnectionId "validation-work-injury")
  $creditEvents = @(Get-CreditDemoEvents -ConnectionId "validation-credit-review")
  foreach ($events in @($workEvents, $creditEvents)) {
    $keys = @($events | ForEach-Object { [string]$_.idempotencyKey })
    if (($keys | Sort-Object -Unique).Count -ne $keys.Count) {
      throw "Demo event validation found duplicate idempotency keys"
    }
  }
  $untitledCreditEvents = @($creditEvents | Where-Object { [string]$_.payload.title -notlike "信贷案件*" })
  if ($untitledCreditEvents.Count -gt 0) {
    throw "Credit review demo events must define a Chinese business title"
  }
  [ordered]@{
    valid = $true
    knowledgeLibraries = @(
      [ordered]@{
        namespace = $workInjuryNamespace
        sourceCount = $workSources.Count
        sourceChunkCount = ($workSources | ForEach-Object { [int]$_.sourceChunkCount } | Measure-Object -Sum).Sum
      },
      [ordered]@{
        namespace = $creditNamespace
        sourceCount = $creditSources.Count
        chunking = "managed_by_sag"
      }
    )
    demoEvents = @(
      [ordered]@{ domain = "work_injury_audit"; count = $workEvents.Count; caseIds = @($workEvents.caseId) },
      [ordered]@{ domain = "credit_review"; count = $creditEvents.Count; caseIds = @($creditEvents.caseId) }
    )
  } | ConvertTo-Json -Depth 20
  return
}

$health = Invoke-TopiaApi GET "/health"
if (-not $health.ok) { throw "OpenTopia server is not healthy" }

$workKnowledge = Import-WorkInjuryKnowledge
$creditKnowledge = Import-CreditKnowledge

$workPython = Join-Path $workInjuryRoot ".venv\Scripts\python.exe"
$workMcpScript = Join-Path $workInjuryRoot "scripts\run_mcp_server.py"
$creditPython = Join-Path $creditRoot ".venv\Scripts\python.exe"
foreach ($path in @($workPython, $workMcpScript, $creditPython)) {
  if (-not (Test-Path -LiteralPath $path -PathType Leaf)) { throw "Required runtime not found: $path" }
}

# Launch the project environments through their base interpreter and prepend the
# venv site-packages explicitly. This avoids coupling the Connection definition
# to a virtual-environment launcher whose base-runtime handoff may be rewritten
# by a Windows process broker. OpenTopia's normal process policy remains in force.
$workBasePython = Get-VenvBasePython $workPython
$creditBasePython = Get-VenvBasePython $creditPython
$workSitePackages = Join-Path $workInjuryRoot ".venv\Lib\site-packages"
$creditSitePackages = Join-Path $creditRoot ".venv\Lib\site-packages"
$creditSource = Join-Path $creditRoot "src"
$workLauncher = "import runpy,sys;sys.path.insert(0,$(ConvertTo-PythonStringLiteral $workSitePackages));sys.argv=[$(ConvertTo-PythonStringLiteral $workMcpScript),'--transport','stdio'];runpy.run_path($(ConvertTo-PythonStringLiteral $workMcpScript),run_name='__main__')"
$creditLauncher = "import runpy,sys;sys.path[:0]=[$(ConvertTo-PythonStringLiteral $creditSource),$(ConvertTo-PythonStringLiteral $creditSitePackages)];sys.argv=['credit-review-mcp','--transport','stdio','--disable-sql'];runpy.run_module('credit_review.mcp_server',run_name='__main__')"

$workServer = Get-OrCreateMcpServer `
  -Name "工伤审核（外部 SAG）" `
  -LegacyNames @("Work Injury Audit (External SAG)") `
  -Command $workBasePython `
  -Arguments @("-c", $workLauncher) `
  -WorkingDirectory $workInjuryRoot
$creditServer = Get-OrCreateMcpServer `
  -Name "信贷审核（外部 SAG）" `
  -LegacyNames @("Credit Review (External SAG)") `
  -Command $creditBasePython `
  -Arguments @("-c", $creditLauncher) `
  -WorkingDirectory $creditRoot

$workAccess = Get-ConnectionAccess $workServer @(
  "audit_demo_case_for_external_sag",
  "get_catalog_stats",
  "search_catalog",
  "match_expense_line"
)
$creditAccess = Get-ConnectionAccess $creditServer @("run_case_for_external_sag")
$workDemoEvents = @(Get-WorkInjuryDemoEvents -ConnectionId $workAccess.ConnectionId)
$creditDemoEvents = @(Get-CreditDemoEvents -ConnectionId $creditAccess.ConnectionId)

$workSummarySchema = @{
  type = "object"
  title = "摘要"
  properties = [ordered]@{
    region = @{ type = "string"; title = "地区" }
    accidentDate = @{ type = "string"; title = "事故日期" }
    expenseTotal = @{ type = "number"; title = "费用总额" }
    expenseCount = @{ type = "integer"; title = "费用项目数" }
    materialCount = @{ type = "integer"; title = "材料数量" }
    recognizedDiagnosisCount = @{ type = "integer"; title = "已认定诊断数" }
  }
  additionalProperties = $false
}
$workOutputSchema = @{
  type = "object"
  properties = [ordered]@{
    authority_verification = @{ type = "object"; title = "权威性核验" }
    automatic_benefit_decision = @{ type = @("string", "null"); title = "自动待遇决定" }
    case_id = @{ type = "string"; title = "案件 ID" }
    decision_boundary = @{ type = "string"; title = "决策边界" }
    findings = @{ type = "array"; title = "审核发现"; items = @{} }
    human_review_required = @{ type = "boolean"; title = "需要人工复核" }
    policy_evidence = @{ type = "array"; title = "政策依据"; items = @{} }
    policy_evidence_assessment = @{ type = "string"; title = "政策依据评估" }
    recommended_review_actions = @{ type = "array"; title = "建议复核操作"; items = @{} }
    review_status = @{ type = "string"; title = "复核状态" }
    review_type = @{ type = "string"; title = "复核类型" }
    summary = @{ type = "object"; title = "摘要" }
  }
  additionalProperties = $true
}
$creditSummarySchema = @{
  type = "object"
  title = "摘要"
  properties = [ordered]@{
    loanAmount = @{ type = @("number", "null"); title = "贷款金额" }
    loanTermMonths = @{ type = @("integer", "null"); title = "贷款期限（月）" }
    loanPurpose = @{ type = "string"; title = "贷款用途" }
    documentCount = @{ type = "integer"; title = "材料数量" }
    documentTypes = @{ type = "array"; title = "材料类型"; items = @{ type = "string" } }
  }
  additionalProperties = $false
}
$creditOutputSchema = @{
  type = "object"
  properties = [ordered]@{
    authority_verification = @{ type = "object"; title = "权威性核验" }
    automatic_credit_decision = @{ type = @("string", "null"); title = "自动授信决定" }
    case_id = @{ type = "string"; title = "案件 ID" }
    cashflow_analysis = @{ type = @("object", "string"); title = "现金流分析" }
    decision_boundary = @{ type = "string"; title = "决策边界" }
    findings = @{ type = "array"; title = "审核发现"; items = @{} }
    human_review_required = @{ type = "boolean"; title = "需要人工复核" }
    policy_evidence = @{ type = "array"; title = "政策依据"; items = @{} }
    policy_evidence_assessment = @{ type = "string"; title = "政策依据评估" }
    recommended_review_actions = @{ type = "array"; title = "建议复核操作"; items = @{} }
    review_status = @{ type = "string"; title = "复核状态" }
    review_type = @{ type = "string"; title = "复核类型" }
    summary = @{ type = "object"; title = "摘要" }
  }
  additionalProperties = $true
}

$workDomain = New-AgentTemplate `
  -TemplateId "audit.work-injury.domain" `
  -Name "工伤领域审核智能体" `
  -Instructions "只读取事件中的 caseId，并仅针对该案件调用 audit_demo_case_for_external_sag。返回领域事实、结构化目录匹配、规则发现和外部知识检索计划。目录工具只能用于核验工具已返回的费用匹配；不得枚举无关案件，不得调用或声称使用旧版项目 RAG。所有面向审核人员的说明使用简体中文。" `
  -ConnectionAccess $workAccess
$workEvidence = New-AgentTemplate `
  -TemplateId "audit.work-injury.evidence" `
  -Name "工伤 SAG 依据检索智能体" `
  -Instructions "使用 library_search 为每项工伤审核发现检索政策依据。只能引用工具实际返回的依据，不得检索模板冻结命名空间以外的内容；所有面向审核人员的摘要和说明使用简体中文。" `
  -KnowledgeNamespace $workInjuryNamespace
$workReport = New-AgentTemplate `
  -TemplateId "audit.work-injury.report" `
  -Name "工伤复核报告智能体" `
  -Instructions "将结构化发现和 SAG 政策依据整理为人工复核草案，并保留依据 ID 与来源。输出必须是 JSON 对象，固定使用 authority_verification、automatic_benefit_decision、case_id、decision_boundary、findings、human_review_required、policy_evidence、policy_evidence_assessment、recommended_review_actions、review_status、review_type、summary 字段。所有面向人员的结论、说明和行动建议直接使用简体中文；review_type 填写“工伤医疗费用审核”，review_status 填写“待人工复核”，automatic_benefit_decision 必须为 null。不得作出自动待遇决定。"

$creditDomain = New-AgentTemplate `
  -TemplateId "audit.credit.domain" `
  -Name "信贷领域审核智能体" `
  -Instructions "只读取事件中的 caseId，并仅针对该案件调用 run_case_for_external_sag。返回材料事实、现金流分析、规则风险和政策检索计划。不得枚举无关案件，不得调用旧版 RAG，也不得使用只读 SQL；所有面向审核人员的说明使用简体中文。" `
  -ConnectionAccess $creditAccess
$creditEvidence = New-AgentTemplate `
  -TemplateId "audit.credit.evidence" `
  -Name "信贷 SAG 依据检索智能体" `
  -Instructions "针对每项信贷风险使用 library_search 检索依据。区分公开资料与 synthetic_internal_sop，不得检索模板冻结命名空间以外的内容；所有面向审核人员的摘要和说明使用简体中文。" `
  -KnowledgeNamespace $creditNamespace
$creditReport = New-AgentTemplate `
  -TemplateId "audit.credit.report" `
  -Name "信贷复核报告智能体" `
  -Instructions "根据事实、风险和 SAG 引用生成信贷人工复核草案。输出必须是 JSON 对象，固定使用 authority_verification、automatic_credit_decision、case_id、cashflow_analysis、decision_boundary、findings、human_review_required、policy_evidence、policy_evidence_assessment、recommended_review_actions、review_status、review_type、summary 字段。所有面向人员的结论、说明和行动建议直接使用简体中文；review_type 填写“信贷材料审核”，review_status 填写“待人工复核”，automatic_credit_decision 必须为 null。不得决定批准、拒绝、额度或定价。"

$workFlow = New-AuditFlow `
  -FlowId "audit-work-injury" `
  -Name "工伤医疗费用审核" `
  -DomainTemplate $workDomain `
  -EvidenceTemplate $workEvidence `
  -ReportTemplate $workReport `
  -EventSource "audit.work-injury" `
  -EventType "case.submitted" `
  -TestCaseId "SZ-GS-2026-0002" `
  -InputSummarySchema $workSummarySchema `
  -OutputSchema $workOutputSchema `
  -DemoEvents $workDemoEvents
$creditFlow = New-AuditFlow `
  -FlowId "audit-credit-review" `
  -Name "信贷材料审核" `
  -DomainTemplate $creditDomain `
  -EvidenceTemplate $creditEvidence `
  -ReportTemplate $creditReport `
  -EventSource "audit.credit-review" `
  -EventType "case.submitted" `
  -TestCaseId "case_income_mismatch" `
  -InputSummarySchema $creditSummarySchema `
  -OutputSchema $creditOutputSchema `
  -DemoEvents $creditDemoEvents

$caseTest = $null
if ($TestQueuedCaseId) {
  if ($SkipSeedEvents) { throw "-TestQueuedCaseId requires seeded Demo events" }
  $caseTest = Test-QueuedAuditCase -Flows @($workFlow, $creditFlow) -CaseId $TestQueuedCaseId
}

[ordered]@{
  configuredAt = [DateTime]::UtcNow.ToString("o")
  knowledgeLibraries = @($workKnowledge, $creditKnowledge)
  connections = @{
    workInjury = @{
      connectionId = $workAccess.ConnectionId
      boundary = "case audit and structured NHSA catalog lookup"
    }
    creditReview = @{
      connectionId = $creditAccess.ConnectionId
      boundary = "event-scoped credit case review"
    }
  }
  templates = @(
    "$($workDomain.templateId)@$($workDomain.version)",
    "$($workEvidence.templateId)@$($workEvidence.version)",
    "$($workReport.templateId)@$($workReport.version)",
    "$($creditDomain.templateId)@$($creditDomain.version)",
    "$($creditEvidence.templateId)@$($creditEvidence.version)",
    "$($creditReport.templateId)@$($creditReport.version)"
  )
  flows = @($workFlow, $creditFlow)
  testedQueuedCase = $caseTest
} | ConvertTo-Json -Depth 100
