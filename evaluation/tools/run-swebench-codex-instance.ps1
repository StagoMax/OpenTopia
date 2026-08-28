param(
    [Parameter(Mandatory = $true)]
    [string]$PlanRoot,

    [Parameter(Mandatory = $true)]
    [string]$Instance,

    [string]$RunsDirectoryName = 'swebench-codex-account-default-20260828',

    [ValidateRange(180, 3600)]
    [int]$RunTimeoutSeconds = 1800
)

$ErrorActionPreference = 'Stop'
$env:PYTHONUTF8 = '1'
$sourceRoot = (Resolve-Path (Join-Path $PSScriptRoot '..\..')).Path
$env:PYTHONPATH = $sourceRoot
$python = Join-Path $PlanRoot 'runtime\swebench-py311\Scripts\python.exe'
$instances = Join-Path $PlanRoot 'swebench-verified-selected-instances-78f471bf.jsonl'
$agentRunner = Join-Path $sourceRoot 'evaluation\integrations\swebench\run_codex_instance.py'
$officialScorer = Join-Path $sourceRoot 'evaluation\integrations\swebench\score_opentopia_prediction.py'
foreach ($path in @($python, $instances, $agentRunner, $officialScorer)) {
    if (-not (Test-Path -LiteralPath $path)) { throw "Required evaluation input is missing: $path" }
}

$instanceDir = Join-Path (Join-Path $PlanRoot $RunsDirectoryName) $Instance
$logsDir = Join-Path $instanceDir 'agent-logs'
$prediction = Join-Path $instanceDir 'prediction.jsonl'
$runLabel = "codex-default-$Instance"
$score = Join-Path $instanceDir 'official-score-lf-fixed.json'
New-Item -ItemType Directory -Force -Path $instanceDir | Out-Null

$agentArguments = @(
    $agentRunner,
    '--instances', $instances,
    '--instance-id', $Instance,
    '--output', $prediction,
    '--logs-dir', $logsDir,
    '--run-label', $runLabel,
    '--run-timeout-seconds', $RunTimeoutSeconds
)
& $python @agentArguments 1>> (Join-Path $instanceDir 'agent.stdout.log') 2>> (Join-Path $instanceDir 'agent.stderr.log')
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$scoreArguments = @(
    $officialScorer,
    '--instances', $instances,
    '--instance-id', $Instance,
    '--prediction', $prediction,
    '--run-id', $runLabel,
    '--timeout-seconds', '1800',
    '--output', $score
)
& $python @scoreArguments 1>> (Join-Path $instanceDir 'score.stdout.log') 2>> (Join-Path $instanceDir 'score.stderr.log')
exit $LASTEXITCODE
