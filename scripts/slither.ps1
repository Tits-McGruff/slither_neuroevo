param(
  [switch]$play,
  [switch]$shutdown
)

$ErrorActionPreference = "Stop"

# The PS1 lives in .\scripts; repo root is its parent directory.
$ScriptsDir = Split-Path -Parent $MyInvocation.MyCommand.Path
$RepoRoot   = Split-Path -Parent $ScriptsDir
Set-Location $RepoRoot

# ============================================================
# Output helpers
# ============================================================

function Write-Info($msg) { Write-Host ("[INFO] " + $msg) }
function Write-Err($msg)  { Write-Host ("[ERROR] " + $msg) }

# ============================================================
# Command availability
# ============================================================

function Require-Command([string]$name, [string]$hint) {
  if (-not (Get-Command $name -ErrorAction SilentlyContinue)) {
    Write-Err "$name is not installed or not in PATH. $hint"
    exit 1
  }
}

# ============================================================
# Minimal TOML reader for just the fields we care about
# Works even if node_modules is broken (important for shutdown).
# ============================================================

function Read-ConfigToml {
  $cfg = [ordered]@{
    host        = "127.0.0.1"
    port        = 5174
    uiHost      = "127.0.0.1"
    uiPort      = 5173
    publicWsUrl = ""
  }

  $path = Join-Path $RepoRoot "server\config.toml"
  if (-not (Test-Path $path)) { return $cfg }

  $lines = Get-Content $path -ErrorAction SilentlyContinue
  foreach ($raw in $lines) {
    $line = $raw.Trim()
    if (-not $line) { continue }
    if ($line.StartsWith("#")) { continue }

    # Strip trailing comments, assuming config values don't contain '#'
    $hash = $line.IndexOf("#")
    if ($hash -ge 0) { $line = $line.Substring(0, $hash).Trim() }
    if (-not $line) { continue }

    if ($line -match '^\s*([A-Za-z0-9_]+)\s*=\s*(.+?)\s*$') {
      $k = $Matches[1]
      $v = $Matches[2].Trim()

      # Strings are quoted
      if ($v -match '^"(.*)"$') { $val = $Matches[1] } else { $val = $v }

      switch ($k) {
        "host"        { $cfg.host = [string]$val }
        "uiHost"      { $cfg.uiHost = [string]$val }
        "publicWsUrl" { $cfg.publicWsUrl = [string]$val }
        "port"        { if ($val -match '^\d+$') { $cfg.port = [int]$val } }
        "uiPort"      { if ($val -match '^\d+$') { $cfg.uiPort = [int]$val } }
      }
    }
  }

  return $cfg
}

# ============================================================
# IP enumeration (Windows-native)
# ============================================================

function Get-NonLoopbackIPv4 {
  $ips = @()

  try {
    $addrs = Get-NetIPAddress -AddressFamily IPv4 -ErrorAction Stop |
      Where-Object {
        $_.IPAddress -and
        $_.IPAddress -notlike "127.*" -and
        $_.IPAddress -ne "0.0.0.0" -and
        $_.IPAddress -notlike "169.254.*"
      } |
      Select-Object -ExpandProperty IPAddress
    $ips = @($addrs)
  } catch {
    $text = ipconfig
    foreach ($m in ($text | Select-String -Pattern 'IPv4 Address[^\:]*:\s*([0-9\.]+)')) {
      $ip = $m.Matches[0].Groups[1].Value
      if ($ip -and $ip -notlike "127.*" -and $ip -notlike "169.254.*" -and $ip -ne "0.0.0.0") {
        $ips += $ip
      }
    }
  }

  $ips = $ips | Where-Object { $_ -match '^\d+\.\d+\.\d+\.\d+$' } | Sort-Object -Unique

  function Score-IP([string]$ip) {
    if ($ip -match '^10\.') { return 0 }
    if ($ip -match '^192\.168\.') { return 1 }
    if ($ip -match '^172\.(\d+)\.') {
      $n = [int]$Matches[1]
      if ($n -ge 16 -and $n -le 31) { return 2 }
    }
    return 9
  }

  return $ips | Sort-Object @{Expression={ Score-IP $_ }; Ascending=$true}, @{Expression={$_}; Ascending=$true}
}

# ============================================================
# Node dependency install policy (matches play.sh)
# ============================================================

function Ensure-Dependencies {
  if (-not (Test-Path (Join-Path $RepoRoot "package.json"))) {
    Write-Err "package.json not found in $RepoRoot"
    exit 1
  }

  $need = $false
  if (-not (Test-Path (Join-Path $RepoRoot "node_modules"))) {
    $need = $true
  } else {
    & node -e "require.resolve('smol-toml')" *> $null
    if ($LASTEXITCODE -ne 0) { $need = $true }
  }

  if ($need) {
    Write-Host ""
    Write-Host "[SETUP] Installing dependencies..."
    Write-Host ""

    if (Test-Path (Join-Path $RepoRoot "package-lock.json")) {
      & npm ci
      if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Err "Failed to install dependencies (npm ci)."
        exit 1
      }
    } else {
      & npm install
      if ($LASTEXITCODE -ne 0) {
        Write-Host ""
        Write-Err "Failed to install dependencies (npm install)."
        exit 1
      }
    }

    Write-Host ""
    Write-Host "[SUCCESS] Dependencies installed!"
  }
}

# ============================================================
# PID/log helpers (repo root)
# ============================================================

function Read-PidFile([string]$file) {
  $path = Join-Path $RepoRoot $file
  if (-not (Test-Path $path)) { return $null }
  $t = (Get-Content $path -ErrorAction SilentlyContinue | Select-Object -First 1)
  if (-not $t) { return $null }
  $t = $t.Trim()
  if ($t -match '^\d+$') { return [int]$t }
  return $null
}

function Write-PidFile([string]$file, [int]$pid) {
  $path = Join-Path $RepoRoot $file
  Set-Content -Path $path -Value $pid -NoNewline
}

function Remove-PidFiles {
  foreach ($f in @("server.pid","dev.pid")) {
    Remove-Item (Join-Path $RepoRoot $f) -Force -ErrorAction SilentlyContinue
  }
}

function Process-Exists([int]$pid) {
  try { return [bool](Get-Process -Id $pid -ErrorAction Stop) } catch { return $false }
}

# ============================================================
# Detached start (Windows semantics)
# ============================================================

function Start-Detached(
  [string]$name,
  [string[]]$npmArgs,
  [string]$pidFile,
  [string]$logFile
) {
  $pidPath = Join-Path $RepoRoot $pidFile
  $logPath = Join-Path $RepoRoot $logFile

  $old = Read-PidFile $pidFile
  if ($old -and (Process-Exists $old)) {
    Write-Info "$name already running with PID $old"
    return $old
  } elseif (Test-Path $pidPath) {
    Remove-Item $pidPath -Force -ErrorAction SilentlyContinue
  }

  Write-Host ""
  Write-Host "Starting $name (detached)..."
  Write-Host ""

  $p = Start-Process -FilePath "npm" -ArgumentList $npmArgs -WorkingDirectory $RepoRoot `
        -RedirectStandardOutput $logPath -RedirectStandardError $logPath -WindowStyle Hidden -PassThru

  Write-PidFile $pidFile $p.Id

  $ok = $false
  for ($i=0; $i -lt 20; $i++) {
    if (Process-Exists $p.Id) { $ok = $true; break }
    Start-Sleep -Milliseconds 500
  }

  if (-not $ok) {
    Write-Host ""
    Write-Err "$name exited during startup. Check $logFile for the reason."
    exit 1
  }

  return $p.Id
}

# ============================================================
# Listener discovery by port (Windows-native)
# ============================================================

function Listener-PidsOnPort([int]$port) {
  $pids = @()
  try {
    $pids = Get-NetTCPConnection -LocalPort $port -State Listen -ErrorAction Stop |
      Select-Object -ExpandProperty OwningProcess
  } catch {
    $pids = @()
    $re = ":" + $port + "\s"
    foreach ($line in (netstat -ano -p tcp | Select-String $re)) {
      $parts = ($line.ToString() -split "\s+") | Where-Object { $_ }
      if ($parts.Count -ge 5) { $pids += $parts[-1] }
    }
  }
  return $pids | Where-Object { $_ -match '^\d+$' } | ForEach-Object { [int]$_ } | Sort-Object -Unique
}

# ============================================================
# Repo guard (repo root path)
# ============================================================

function Pid-BelongsToRepo([int]$pid) {
  try {
    $p = Get-CimInstance Win32_Process -Filter ("ProcessId=" + $pid) -ErrorAction Stop
    if (-not $p) { return $false }
    if (-not $p.CommandLine) { return $false }
    return ($p.CommandLine -like ("*" + $RepoRoot + "*"))
  } catch {
    return $false
  }
}

# ============================================================
# Stop process tree (Windows semantics)
# ============================================================

function Stop-PidTree([string]$name, [int]$pid) {
  if (-not (Process-Exists $pid)) { return $true }

  if (-not (Pid-BelongsToRepo $pid)) {
    Write-Info "$name: PID $pid does not look like it belongs to $RepoRoot, skipping."
    return $false
  }

  Write-Info "Stopping $name PID $pid..."
  & taskkill /PID $pid /T *> $null

  $stopped = $false
  for ($i=0; $i -lt 5; $i++) {
    if (-not (Process-Exists $pid)) { $stopped = $true; break }
    Start-Sleep -Seconds 1
  }

  if (-not $stopped) {
    Write-Info "$name did not exit, sending force kill..."
    & taskkill /F /PID $pid /T *> $null
    for ($i=0; $i -lt 5; $i++) {
      if (-not (Process-Exists $pid)) { $stopped = $true; break }
      Start-Sleep -Seconds 1
    }
  }

  if ($stopped) {
    Write-Info "$name stopped."
    return $true
  } else {
    Write-Err "$name PID $pid is still running."
    return $false
  }
}

# ============================================================
# Optional: derive ports from logs (helps if config differs from reality)
# ============================================================

function Get-PortsFromLogs {
  $ports = @()

  $devLog = Join-Path $RepoRoot "dev.log"
  if (Test-Path $devLog) {
    $txt = Get-Content $devLog -ErrorAction SilentlyContinue
    foreach ($m in ($txt | Select-String -Pattern 'http://localhost:(\d+)' -AllMatches)) {
      foreach ($mm in $m.Matches) { $ports += [int]$mm.Groups[1].Value }
    }
  }

  $srvLog = Join-Path $RepoRoot "server.log"
  if (Test-Path $srvLog) {
    $txt = Get-Content $srvLog -ErrorAction SilentlyContinue
    foreach ($m in ($txt | Select-String -Pattern 'listening on\s*:([0-9]+)' -AllMatches)) {
      foreach ($mm in $m.Matches) { $ports += [int]$mm.Groups[1].Value }
    }
  }

  return $ports | Sort-Object -Unique
}

# ============================================================
# Print connection details
# ============================================================

function Print-ConnectionDetails($cfg) {
  $ips = Get-NonLoopbackIPv4

  Write-Host ""
  Write-Host "Connection details:"
  Write-Host ""
  Write-Host ("UI Local:       http://localhost:{0}/" -f $cfg.uiPort)
  Write-Host ("Server Local:   http://localhost:{0}/" -f $cfg.port)
  Write-Host ""

  $uiHosts = @()
  $srvHosts = @()

  if ($cfg.uiHost -eq "0.0.0.0") { $uiHosts = @($ips) }
  elseif ($cfg.uiHost -and $cfg.uiHost -ne "127.0.0.1" -and $cfg.uiHost.ToLower() -ne "localhost") { $uiHosts = @($cfg.uiHost) }

  if ($cfg.host -eq "0.0.0.0") { $srvHosts = @($ips) }
  elseif ($cfg.host -and $cfg.host -ne "127.0.0.1" -and $cfg.host.ToLower() -ne "localhost") { $srvHosts = @($cfg.host) }

  foreach ($h in $uiHosts) { Write-Host ("UI Network:     http://{0}:{1}/" -f $h, $cfg.uiPort) }
  if ($uiHosts.Count -gt 0) { Write-Host "" }
  foreach ($h in $srvHosts) { Write-Host ("Server Network: http://{0}:{1}/" -f $h, $cfg.port) }
  if ($srvHosts.Count -gt 0) { Write-Host "" }

  if ($cfg.publicWsUrl) {
    Write-Host ("WebSocket Public: {0}" -f $cfg.publicWsUrl)
  } else {
    Write-Host ("WebSocket Local:   ws://localhost:{0}/" -f $cfg.port)
    $wsHosts = @()
    if ($uiHosts.Count -gt 0) { $wsHosts = $uiHosts }
    elseif ($srvHosts.Count -gt 0) { $wsHosts = $srvHosts }
    foreach ($h in $wsHosts) { Write-Host ("WebSocket Network: ws://{0}:{1}/" -f $h, $cfg.port) }
  }

  Write-Host ""
  Write-Host "Open the UI URL in your browser."
  Write-Host ""
}

# ============================================================
# PLAY
# ============================================================

function Do-Play {
  Write-Host "========================================"
  Write-Host "Slither Neuroevolution Launcher"
  Write-Host "========================================"

  Require-Command node "Please install from https://nodejs.org/"
  Require-Command npm  "It normally ships with Node.js; reinstall from https://nodejs.org/"

  Ensure-Dependencies
  $cfg = Read-ConfigToml

  $serverPid = Start-Detached "Simulation Server" @("run","server") "server.pid" "server.log"
  $devPid    = Start-Detached "Vite Dev Server"   @("run","dev","--","--force") "dev.pid" "dev.log"

  Write-Host ""
  Write-Host ("[OK] Simulation server running   PID: {0}   Log: server.log" -f $serverPid)
  Write-Host ("[OK] Vite dev server running     PID: {0}   Log: dev.log" -f $devPid)
  Write-Host ""

  # Rewrite pid files to actual listener PIDs when possible, reduces wrapper-PID mismatch.
  $uiListener = Listener-PidsOnPort $cfg.uiPort | Select-Object -First 1
  if ($uiListener) { Write-PidFile "dev.pid" $uiListener }

  $srvListener = Listener-PidsOnPort $cfg.port | Select-Object -First 1
  if ($srvListener) { Write-PidFile "server.pid" $srvListener }

  Print-ConnectionDetails $cfg
}

# ============================================================
# SHUTDOWN
# ============================================================

function Do-Shutdown {
  Write-Host "========================================"
  Write-Host "Slither Neuroevolution Shutdown"
  Write-Host "========================================"

  $cfg = Read-ConfigToml

  # Step 1: best-effort stop pidfile targets
  $serverPid = Read-PidFile "server.pid"
  if ($serverPid) { Stop-PidTree "Simulation Server (pidfile)" $serverPid | Out-Null }

  $devPid = Read-PidFile "dev.pid"
  if ($devPid) { Stop-PidTree "Vite Dev Server (pidfile)" $devPid | Out-Null }

  # Step 2: stop real listeners by config ports plus log-derived ports
  $ports = @($cfg.uiPort, $cfg.port) + (Get-PortsFromLogs)
  $ports = $ports | Where-Object { $_ -is [int] -and $_ -gt 0 } | Sort-Object -Unique

  $listenerPids = @()
  foreach ($pt in $ports) { $listenerPids += (Listener-PidsOnPort $pt) }
  $listenerPids = $listenerPids | Sort-Object -Unique

  $guarded = @()
  foreach ($pid in $listenerPids) {
    if (Pid-BelongsToRepo $pid) { $guarded += $pid }
  }
  $guarded = $guarded | Sort-Object -Unique

  if ($guarded.Count -gt 0) {
    Write-Info ("Detected listener processes on ports {0}, stopping them..." -f ($ports -join ","))
    foreach ($pid in $guarded) { Stop-PidTree "Listener" $pid | Out-Null }
  }

  # Step 3: verify nothing repo-owned still listens on those ports
  $left = @()
  foreach ($pt in $ports) {
    foreach ($pid in (Listener-PidsOnPort $pt)) {
      if (Pid-BelongsToRepo $pid) { $left += $pid }
    }
  }
  $left = $left | Sort-Object -Unique

  if ($left.Count -eq 0) {
    Remove-PidFiles
    Write-Host "[OK] Shutdown complete."
  } else {
    Write-Err ("Some repo processes are still listening, remaining PIDs: {0}" -f ($left -join " "))
    Write-Info "Keeping pid files so shutdown can be retried."
    exit 1
  }
}

# ============================================================
# Entry
# ============================================================

if ($play)     { Do-Play; exit 0 }
if ($shutdown) { Do-Shutdown; exit 0 }

Write-Err "Missing mode. Use play.bat or shutdown.bat from the repo root."
exit 1
