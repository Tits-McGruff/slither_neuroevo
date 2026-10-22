"""Bounded system-wide resource observations beside a qualification run.

Windows observations include every already-running WSL distribution. GPU totals
are system-wide; process memory unavailable under WDDM stays unavailable. Process
names and resource counters are collected without command lines or environments.
This observer does not start stopped distributions or change process priorities.
"""
import argparse
import csv
from datetime import datetime, timezone
import hashlib
import io
import json
import os
from pathlib import Path
import platform
import subprocess
import time


def decode_output(raw):
    """WSL inventory can be UTF-16LE even when the Windows console uses UTF-8."""
    return raw.decode("utf-16le" if b"\0" in raw else "utf-8", errors="replace").strip("\ufeff \r\n")


def command_output(command):
    """Bound subprocess time and retained output; failure is an explicit missing observation."""
    try:
        result = subprocess.run(command, capture_output=True, timeout=8, check=False)
    except (OSError, subprocess.TimeoutExpired) as error:
        return {"available": False, "error": type(error).__name__}
    if result.returncode or len(result.stdout) > 262144:
        return {"available": False, "exitCode": result.returncode,
                "oversized": len(result.stdout) > 262144}
    return {"available": True, "text": decode_output(result.stdout)}


def csv_observation(command, process_names=False):
    """Retain explicit vendor unavailable values and remove process installation paths."""
    result = command_output(command)
    if not result["available"]:
        return result
    rows = list(csv.DictReader(io.StringIO(result["text"]), skipinitialspace=True))
    if process_names:
        for row in rows:
            if "process_name" in row:
                row["process_name"] = row["process_name"].replace("\\", "/").rsplit("/", 1)[-1]
    return {"available": True, "rows": rows}


def windows_resources():
    """Read system CPU/memory and the busiest process counters, without private arguments."""
    script = r"""
$taskCpu = Get-CimInstance Win32_PerfFormattedData_PerfOS_Processor -Filter "Name='_Total'"
$taskMemory = Get-CimInstance Win32_OperatingSystem
$taskProcesses = @(Get-CimInstance Win32_PerfFormattedData_PerfProc_Process |
    Where-Object { $_.Name -ne '_Total' -and $_.Name -ne 'Idle' } |
    Sort-Object PercentProcessorTime -Descending |
    Select-Object -First 32 Name,IDProcess,PercentProcessorTime,WorkingSetPrivate)
[pscustomobject]@{cpuPercent=$taskCpu.PercentProcessorTime;
    totalMemoryKiB=$taskMemory.TotalVisibleMemorySize;
    freeMemoryKiB=$taskMemory.FreePhysicalMemory; processes=$taskProcesses} |
    ConvertTo-Json -Depth 4 -Compress
"""
    result = command_output(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", script])
    if not result["available"]:
        return result
    try:
        return {"available": True, **json.loads(result["text"])}
    except (ValueError, TypeError):
        return {"available": False, "error": "invalid Windows counter response"}


def running_wsl_resources():
    """Observe only running distributions; querying resources never boots a stopped one."""
    inventory = command_output(["wsl.exe", "--list", "--running", "--quiet"])
    if not inventory["available"]:
        return inventory
    observations = []
    for name in inventory["text"].splitlines()[:16]:
        name = name.strip()
        if not name:
            continue
        # COMM contains only executable names. %CPU is ps lifetime CPU, not an interval sample.
        result = command_output(["wsl.exe", "--distribution", name, "--exec", "sh", "-c",
                                 "LC_ALL=C; export LC_ALL; cat /proc/loadavg; "
                                 "head -5 /proc/meminfo; "
                                 "ps -eo pid,comm,pcpu,pmem,rss --sort=-pcpu | head -33"])
        observations.append({"distribution": name, **result})
    return {"available": True, "distributions": observations}


def linux_resources():
    """Read kernel load/memory and named processes on the native Linux qualification host."""
    result = command_output(["sh", "-c", "LC_ALL=C; export LC_ALL; cat /proc/loadavg; "
                             "head -5 /proc/meminfo; "
                             "ps -eo pid,comm,pcpu,pmem,rss --sort=-pcpu | head -33"])
    return result


def snapshot():
    """Bracket collection wall time and keep attribution limits visible in every observation."""
    started = time.monotonic()
    result = {"schemaVersion": 1, "utc": datetime.now(timezone.utc).isoformat(),
              "platform": platform.system(),
              "gpu": csv_observation([
                  "nvidia-smi", "--query-gpu=name,utilization.gpu,utilization.memory,"
                  "memory.used,memory.total,power.draw,temperature.gpu", "--format=csv"]),
              "gpuProcesses": csv_observation([
                  "nvidia-smi", "--query-compute-apps=pid,process_name,used_gpu_memory",
                  "--format=csv"], process_names=True),
              "attribution": "GPU totals include all workloads; unavailable per-process "
                             "GPU memory is unknown. Linux ps CPU is a lifetime average."}
    result["host"] = windows_resources() if os.name == "nt" else linux_resources()
    if os.name == "nt":
        result["wsl"] = running_wsl_resources()
    result["collectionSeconds"] = time.monotonic() - started
    return result


def process_alive(pid):
    """Observe one requested process without signals, priorities or child ownership changes."""
    if os.name != "nt":
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        except PermissionError:
            return True
        return True
    import ctypes
    from ctypes import wintypes
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
    kernel.OpenProcess.restype = wintypes.HANDLE
    kernel.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    handle = kernel.OpenProcess(0x1000, False, pid)
    if not handle:
        # Access denied is unavailable observation, not evidence that the owner exited.
        if ctypes.get_last_error() == 5:
            return True
        return False
    try:
        status = wintypes.DWORD()
        if not kernel.GetExitCodeProcess(handle, ctypes.byref(status)):
            raise ctypes.WinError(ctypes.get_last_error())
        return status.value == 259
    finally:
        kernel.CloseHandle(handle)


def main():
    """Create one bounded sidecar; never overwrite a prior measurement or manage other tasks."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--interval-seconds", type=int, default=60)
    parser.add_argument("--samples", type=int, default=1)
    parser.add_argument("--watch-pid", type=int)
    args = parser.parse_args()
    if not 30 <= args.interval_seconds <= 300 or not 1 <= args.samples <= 1440:
        parser.error("interval must be 30..300 seconds and samples 1..1440")
    if args.watch_pid is not None and not 1 <= args.watch_pid < 2**32:
        parser.error("watch PID must be a positive Uint32")
    source_sha = hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    with args.output.open("x", encoding="utf-8") as stream:
        for index in range(args.samples):
            if args.watch_pid is not None and not process_alive(args.watch_pid):
                break
            started = time.monotonic()
            stream.write(json.dumps({**snapshot(), "sample": index,
                                     "observerSourceSha256": source_sha}, allow_nan=False) + "\n")
            stream.flush()
            if index + 1 < args.samples:
                time.sleep(max(0, args.interval_seconds - (time.monotonic() - started)))


if __name__ == "__main__":
    main()
