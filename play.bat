play.bat:
@echo off
setlocal

:: --------------------------------------------------------------------
:: play.bat
:: --------------------------------------------------------------------
:: Windows entrypoint for starting Slither Neuroevolution.
::
:: Why this file exists:
:: - Many Windows users will double click a .bat, a .ps1 is easier to 
::   maintain  but easier to use incorrectly (right click, 
::   "Run with PowerShell") without required args.
:: - This .bat keeps the user-facing entrypoint simple and consistent.
::
:: What it does:
:: - Changes directory to the repository root (this file's directory).
:: - Invokes the real launcher logic in scripts\slither.ps1 with --play.
::
:: Output, PID files, logs:
:: - server.pid / dev.pid are written in the repo root
:: - server.log / dev.log are written in the repo root
:: --------------------------------------------------------------------

:: Ensure we run from the repo root, even if invoked from another directory.
cd /d "%~dp0" || exit /b 1

:: Call the PowerShell launcher in "play" mode.
:: -NoProfile avoids user profile side effects,
:: -ExecutionPolicy Bypass avoids local policy blocks for this single invocation.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\slither.ps1" -play

:: Propagate the PowerShell exit code back to the caller.
exit /b %errorlevel%
