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
