[CmdletBinding()]
param(
  [string]$BaseUrl = "http://127.0.0.1:8787",
  [string]$ApiToken = $env:OPENTOPIA_API_TOKEN,
  [string]$WorkInjuryProject = "",
  [string]$CreditProject = "",
  [string]$Owner = "audit-platform-admin",
  [string]$Approver = "audit-risk-approver",
  [string]$Environment = "local",
  [switch]$SkipSeedEvents,
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
    [Parameter(Mandatory = $true)][ValidateSet("GET", "POST", "PATCH")][string]$Method,
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
  Invoke-RestMethod @parameters
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
  $task = @(Invoke-TopiaApi GET "/api/human-tasks?status=pending&flowRunId=$($Run.id)") |
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
    [string]$Command,
    [string[]]$Arguments,
    [string]$WorkingDirectory
  )
  $existing = @(Expand-TopiaItems (Invoke-TopiaApi GET "/api/mcp/servers")) |
    Where-Object { $_.server.name -eq $Name } |
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
    description = "OpenTopia audit workflow template: $Name"
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
      idempotencyKey = "demo-event:work-injury:${caseId}:v1"
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
      idempotencyKey = "demo-event:credit-review:${caseId}:v1"
      caseId = $caseId
      payload = [ordered]@{
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
    [Parameter(Mandatory = $true)][object]$Release,
    [Parameter(Mandatory = $true)][object]$Deployment,
    [Parameter(Mandatory = $true)][object[]]$Events
  )
  $caseIds = @($Events | ForEach-Object { [string]$_.caseId })
  if (($caseIds | Sort-Object -Unique).Count -ne $caseIds.Count) {
    throw "Demo event case IDs must be unique for Release $($Release.id)"
  }
  $idempotencyKeys = @($Events | ForEach-Object { [string]$_.idempotencyKey })
  if (($idempotencyKeys | Sort-Object -Unique).Count -ne $idempotencyKeys.Count) {
    throw "Demo event idempotency keys must be unique for Release $($Release.id)"
  }
  $seeded = foreach ($event in $Events) {
    $result = Invoke-TopiaApi POST "/api/workflow-releases/$($Release.id)/invoke" @{
      idempotencyKey = [string]$event.idempotencyKey
      input = $event.payload
    }
    if ($result.invocation.releaseId -ne $Release.id -or $result.invocation.deploymentId -ne $Deployment.id) {
      throw "Demo event '$($event.caseId)' was routed outside its target Release or Deployment"
    }
    if ($result.invocation.status -ne "accepted" -or $result.invocation.flowRunId -or $result.run) {
      throw "Demo event '$($event.caseId)' did not remain pending for human trigger review"
    }
    [ordered]@{
      caseId = [string]$event.caseId
      invocationId = [string]$result.invocation.id
      reused = [bool]$result.reused
    }
  }
  [ordered]@{
    requested = $Events.Count
    accepted = @($seeded).Count
    events = @($seeded)
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
    [object[]]$DemoEvents
  )
  $thread = Invoke-TopiaApi POST "/api/threads" @{
    title = "$Name - reviewed event queue"
    workspaceRoot = $repoRoot
    experienceMode = "flow"
  }
  $inputSchema = @{
    type = "object"
    required = @("caseId")
    properties = @{
      caseId = @{ type = "string" }
      eventKind = @{ type = "string" }
      payloadRef = @{ type = "string" }
      synthetic = @{ type = "boolean" }
      summary = @{ type = "object" }
    }
    additionalProperties = $false
  }
  $objectSchema = @{ type = "object" }
  $node = {
    param($Id, $Label, $Template, $Instructions)
    @{
      id = $Id
      label = $Label
      kind = "agent"
      config = @{
        reference = $Template.templateId
        templateVersion = $Template.version
        instructions = $Instructions
      }
      inputSchema = $objectSchema
      outputSchema = $objectSchema
    }
  }
  $nodes = @(
    (& $node "domain_audit" "Structured audit" $DomainTemplate "Call the approved external-SAG domain audit tool. Never call legacy RAG tools. Preserve caseId, runId, rule findings, evidence IDs, and policy search queries."),
    (& $node "sag_evidence" "SAG policy evidence" $EvidenceTemplate "Call library_search for every policy query from the previous node. Use only the template-frozen namespace and return title, sourcePath, eventId/evidenceId, and a concise excerpt."),
    @{ id = "evidence_validator"; label = "Evidence completeness check"; kind = "validator"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    @{ id = "review_gate"; label = "Human review gate"; kind = "approval"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema },
    (& $node "review_report" "Review report" $ReportTemplate "Merge structured facts, rule findings, and SAG citations into a JSON review draft. This is human review assistance, never an automatic approval, denial, payment, or lending decision."),
    @{ id = "output"; label = "Output pending human review"; kind = "output"; config = @{}; inputSchema = $objectSchema; outputSchema = $objectSchema }
  )
  $edges = @(
    @{ from = "domain_audit"; to = "sag_evidence"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "sag_evidence"; to = "evidence_validator"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "evidence_validator"; to = "review_gate"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "review_gate"; to = "review_report"; allowedFields = @(); dataClassification = "confidential" },
    @{ from = "review_report"; to = "output"; allowedFields = @(); dataClassification = "confidential" }
  )
  $spec = @{
    flowId = $FlowId
    name = $Name
    description = "Events enter a pending queue. Human start approval runs structured review, isolated SAG retrieval, and report drafting."
    owner = $Owner
    categories = @("audit", "human-in-the-loop", "sag")
    source = @{ kind = "natural_language"; description = "Agent + isolated SAG + reviewed event ingress" }
    inputSchema = $inputSchema
    outputSchema = $objectSchema
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
  $definition = Invoke-TopiaApi POST "/api/flow-drafts/$($draft.draft.id)/publish" @{ publishedBy = $Approver }
  $deployment = Invoke-TopiaApi POST "/api/workflow-deployments" @{
    flowId = $definition.flowId
    flowVersion = $definition.version
    name = "$Name - $Environment"
    environment = $Environment
    createdBy = $Owner
    outputReviewPolicy = "always_review_output"
    output = @{ kind = "inbox" }
  }
  $release = Invoke-TopiaApi POST "/api/workflow-releases" @{
    releaseKey = "$FlowId-$($deployment.id.ToString().Substring(0, 8))"
    environment = $Environment
    threadId = $thread.id
    deploymentId = $deployment.id
    trigger = @{
      kind = "event_subscription"
      triggerId = [guid]::NewGuid().ToString()
      source = $EventSource
      eventType = $EventType
    }
    ingressPolicy = "require_review"
    createdBy = $Owner
  }
  $seed = [ordered]@{ requested = 0; accepted = 0; events = @() }
  if (-not $SkipSeedEvents) {
    $seed = Seed-DemoEvents -Release $release -Deployment $deployment -Events $DemoEvents
  }
  @{
    threadId = $thread.id
    flowId = $definition.flowId
    flowVersion = $definition.version
    deploymentId = $deployment.id
    releaseId = $release.id
    ingressPolicy = $release.ingressPolicy
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
  -Name "Work Injury Audit (External SAG)" `
  -Command $workBasePython `
  -Arguments @("-c", $workLauncher) `
  -WorkingDirectory $workInjuryRoot
$creditServer = Get-OrCreateMcpServer `
  -Name "Credit Review (External SAG)" `
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

$workDomain = New-AgentTemplate `
  -TemplateId "audit.work-injury.domain" `
  -Name "Work Injury Domain Audit Agent" `
  -Instructions "Read only the event caseId and call audit_demo_case_for_external_sag for that case. Return domain facts, structured catalog matches, rule findings, and external knowledge queries. Use the catalog tools only to verify a returned expense match. Never enumerate unrelated cases or call or claim to use the legacy project RAG." `
  -ConnectionAccess $workAccess
$workEvidence = New-AgentTemplate `
  -TemplateId "audit.work-injury.evidence" `
  -Name "Work Injury SAG Evidence Agent" `
  -Instructions "Use library_search to retrieve policy evidence for work injury findings. Cite only returned evidence and never search outside the frozen namespace." `
  -KnowledgeNamespace $workInjuryNamespace
$workReport = New-AgentTemplate `
  -TemplateId "audit.work-injury.report" `
  -Name "Work Injury Review Report Agent" `
  -Instructions "Turn structured findings and SAG evidence into a human review draft with evidence IDs and sources. Never make an automatic benefit decision."

$creditDomain = New-AgentTemplate `
  -TemplateId "audit.credit.domain" `
  -Name "Credit Domain Audit Agent" `
  -Instructions "Read only the event caseId and call run_case_for_external_sag for that case. Return material facts, cashflow analysis, rule risks, and the policy search plan. Never enumerate unrelated cases, call the legacy RAG, or use readonly SQL." `
  -ConnectionAccess $creditAccess
$creditEvidence = New-AgentTemplate `
  -TemplateId "audit.credit.evidence" `
  -Name "Credit SAG Evidence Agent" `
  -Instructions "Use library_search for each credit risk. Distinguish public references from synthetic_internal_sop and never search outside the frozen namespace." `
  -KnowledgeNamespace $creditNamespace
$creditReport = New-AgentTemplate `
  -TemplateId "audit.credit.report" `
  -Name "Credit Review Report Agent" `
  -Instructions "Produce a human credit review draft from facts, risks, and SAG citations. Never decide approval, denial, limit, or pricing."

$workFlow = New-AuditFlow `
  -FlowId "audit-work-injury" `
  -Name "Work Injury Medical Expense Review" `
  -DomainTemplate $workDomain `
  -EvidenceTemplate $workEvidence `
  -ReportTemplate $workReport `
  -EventSource "audit.work-injury" `
  -EventType "case.submitted" `
  -TestCaseId "SZ-GS-2026-0002" `
  -DemoEvents $workDemoEvents
$creditFlow = New-AuditFlow `
  -FlowId "audit-credit-review" `
  -Name "Credit Material Review" `
  -DomainTemplate $creditDomain `
  -EvidenceTemplate $creditEvidence `
  -ReportTemplate $creditReport `
  -EventSource "audit.credit-review" `
  -EventType "case.submitted" `
  -TestCaseId "case_income_mismatch" `
  -DemoEvents $creditDemoEvents

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
  workflows = @($workFlow, $creditFlow)
} | ConvertTo-Json -Depth 100
