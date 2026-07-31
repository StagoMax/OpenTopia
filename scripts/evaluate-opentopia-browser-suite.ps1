param(
  [Parameter(Mandatory = $true)][string]$EnvFile,
  [string]$Profile = "AUDIT_COPILOT_LLM",
  [string]$ExpectedModel = "",
  [ValidateRange(1024, 65535)][int]$Port = 8812,
  [ValidateRange(1024, 65535)][int]$BrowserFixturePort = 8999,
  [ValidateRange(1, 100)][int]$Repetitions = 1,
  [string]$OutputDirectory = "",
  [string]$SummaryPath = "",
  [switch]$SkipBuild
)

$ErrorActionPreference = "Stop"

$parameters = @{
  EnvFile = $EnvFile
  Profile = $Profile
  Port = $Port
  BrowserFixturePort = $BrowserFixturePort
  Repetitions = $Repetitions
  SuitePath = "evaluation\examples\opentopia-browser-suite\suite.json"
  BrowserFixture = $true
  SkipBuild = $SkipBuild
}
if ($ExpectedModel) { $parameters.ExpectedModel = $ExpectedModel }
if ($OutputDirectory) { $parameters.OutputDirectory = $OutputDirectory }
if ($SummaryPath) { $parameters.SummaryPath = $SummaryPath }

& "$PSScriptRoot\evaluate-opentopia-tool-suite.ps1" @parameters
exit $LASTEXITCODE
