@echo off
cd /d "%~dp0"
setlocal

echo ========================================
echo Slither Neuroevolution Launcher
echo ========================================

:: 1. Check if Node.js is installed
node -v >nul 2>&1
if %errorlevel% neq 0 (
    echo [ERROR] Node.js is not installed or not in your PATH.
    echo Please download and install it from https://nodejs.org/
    pause
    exit /b 1
)

:: 2. Check if node_modules exists, install if missing
if not exist "node_modules" (
    echo.
    echo [FIRST RUN] Dependencies not found. Installing now...
    echo.
    call npm install
    if %errorlevel% neq 0 (
        echo.
        echo [ERROR] Failed to install dependencies.
        pause
        exit /b 1
    )
    echo.
    echo [SUCCESS] Dependencies installed!
)

:: 3. Start the simulation server in a separate window
echo.
echo Starting Simulation Server...
echo.
start "Slither Server" cmd /c npm run server

:: 4. Run the development server and open browser
echo.
echo Starting Simulation...
echo Your browser should open automatically.
echo.
call npm run dev -- --open --force

:: Pause only if the server crashes unexpectedly
pause
