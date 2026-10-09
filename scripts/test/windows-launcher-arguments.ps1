# Exercise production launcher functions without building or starting an owner database.
param([string]$LauncherPath, [string]$FixtureDirectory)

$ErrorActionPreference = 'Stop'
$tokens = $null
$parseErrors = $null
$syntax = [System.Management.Automation.Language.Parser]::ParseFile($LauncherPath, [ref]$tokens, [ref]$parseErrors)
if ($parseErrors.Count -ne 0) { throw 'Launcher contains PowerShell syntax errors.' }
$definitions = $syntax.FindAll({
  param($node)
  $node -is [System.Management.Automation.Language.FunctionDefinitionAst]
}, $false)
foreach ($definition in $definitions) { Invoke-Expression $definition.Extent.Text }

$RepoRoot = $FixtureDirectory
Set-Location -LiteralPath $RepoRoot
if ((Join-CmdArgs @('run', 'server')) -ne 'run server') { throw 'Server arguments were lost.' }
if ((Join-CmdArgs @('run', 'dev', '--', '--force')) -ne 'run dev -- --force') { throw 'Vite arguments were lost.' }
if ((Join-CmdArgs @('path with spaces', 'a&b')) -ne '"path with spaces" "a&b"') {
  throw 'Arguments requiring cmd quoting were not preserved.'
}

$ownedProcessIds = @()
try {
  $ownedProcessIds += Start-Detached 'Simulation Server' @('run', 'server') 'server.pid' 'server.log'
  $ownedProcessIds += Start-Detached 'Vite Dev Server' @('run', 'dev', '--', '--force') 'dev.pid' 'dev.log'
  foreach ($processId in $ownedProcessIds) { Wait-Process -Id $processId -Timeout 10 -ErrorAction SilentlyContinue }
} finally {
  foreach ($processId in $ownedProcessIds) { Stop-Process -Id $processId -Force -ErrorAction SilentlyContinue }
}

# Reproduce native stderr from a failed polite taskkill without touching a real process.
$env:PATH = "$FixtureDirectory;$env:PATH"
$stubTaskkill = Join-Path $FixtureDirectory 'taskkill.cmd'
if ((Get-Command taskkill).Source -ne $stubTaskkill) { throw 'Shutdown test did not select its owned taskkill stub.' }
function Process-Exists([int]$procId) { return -not (Test-Path -LiteralPath (Join-Path $FixtureDirectory 'forced.flag')) }
function Pid-BelongsToRepo([int]$procId) { return $true }
function Start-Sleep { }
if (-not (Stop-PidTree 'Shutdown fixture' 12345)) { throw 'Native stderr prevented the force-stop fallback.' }
if ($ErrorActionPreference -ne 'Stop') { throw 'Shutdown changed the caller error policy.' }

Remove-Item -LiteralPath (Join-Path $FixtureDirectory 'forced.flag')
$env:SLITHER_STUB_FORCE_FAIL = '1'
if (Stop-PidTree 'Unstoppable fixture' 12345) { throw 'Shutdown reported success while the process was still alive.' }
