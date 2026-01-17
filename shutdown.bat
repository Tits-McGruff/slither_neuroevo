@echo off
setlocal

:: --------------------------------------------------------------------
:: shutdown.bat
:: --------------------------------------------------------------------
:: Windows entrypoint for stopping Slither Neuroevolution.
::
:: Why this file exists:
:: - Provides a safe, predictable way to run the shutdown logic with args.
:: - Avoids people running the PS1 directly without the required mode switch.
::
:: What it does:
:: - Changes directory to the repository root (this file's directory).
:: - Invokes scripts\slither.ps1 with --shutdown.
::
:: Shutdown behavior summary:
:: - Attempts to stop PIDs recorded in server.pid and dev.pid (fast path).
:: - If PID files are stale or point at wrappers, finds the actual listeners on
::   the configured ports and stops those instead (repo-guarded).
:: - Verifies ports are no longer being served by repo processes before printing
::   "Shutdown complete".
:: --------------------------------------------------------------------

:: Ensure we run from the repo root, even if invoked from another directory.
cd /d "%~dp0" || exit /b 1

:: Call the PowerShell launcher in "shutdown" mode.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0scripts\slither.ps1" -shutdown

:: Propagate the PowerShell exit code back to the caller.
exit /b %errorlevel%
