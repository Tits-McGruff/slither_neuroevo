@echo off
setlocal

set "NODE_BIN=%npm_node_execpath%"
if not "%NODE_BIN%"=="" goto run

set "NPM_CLI=%npm_execpath%"
if "%NPM_CLI%"=="" goto fallback

for %%I in ("%NPM_CLI%") do set "NPM_CLI=%%~fI"
for %%I in ("%NPM_CLI%") do set "NPM_BIN=%%~dpI"
for %%I in ("%NPM_BIN%..\..\node.exe") do set "NODE_BIN=%%~fI"

:run
if not "%NODE_BIN%"=="" (
  "%NODE_BIN%" "%~dp0postinstall.cjs"
  exit /b %ERRORLEVEL%
)

:fallback
where node >nul 2>&1
if %ERRORLEVEL%==0 (
  node "%~dp0postinstall.cjs"
  exit /b %ERRORLEVEL%
)

echo Failed to locate node.exe for postinstall.
exit /b 1
