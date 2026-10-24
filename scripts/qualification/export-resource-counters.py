"""Export bounded observer JSONL as publishable aggregate counters.

Local debug inventories are rejected, never redacted into a publishable report.
All retained fields are rebuilt from typed allowlists. In particular, GPU model
names, arbitrary vendor text and arbitrary error/attribution strings are omitted.
The original local sidecar is left untouched; the output must not already exist.
"""
import argparse
from datetime import datetime, timezone
import json
import math
from pathlib import Path
import re

# Match the observer's longest supported run without unbounded input or output.
MAX_SAMPLES = 1440
MAX_LINE_BYTES = 256 * 1024
MAX_INPUT_BYTES = 64 * 1024**2
MAX_OUTPUT_BYTES = 16 * 1024**2
ATTRIBUTION = ("GPU totals include all workloads; per-process ownership "
               "is not inferred from aggregate counters.")
GPU_FIELDS = ("utilization.gpu [%]", "utilization.memory [%]",
              "memory.used [MiB]", "memory.total [MiB]",
              "power.draw [W]", "temperature.gpu")
GPU_UNITS = (" %", " %", " MiB", " MiB", " W", "")
MEMORY_FIELDS = ("MemTotal", "MemFree", "MemAvailable", "Buffers", "Cached")
PRIVATE_FIELDS = frozenset(("hostname", "username", "command", "commandline",
                            "processes", "processname", "pid", "pids",
                            "idprocess", "gpuprocesses", "distribution",
                            "distributionname", "distributionid"))


def require(condition, message):
    """Reject malformed input without echoing private source values."""
    if not condition:
        raise ValueError(message)


def reject_private_fields(value, depth=0):
    """Refuse a debug inventory even if its process-details marker was changed."""
    require(depth <= 12, "resource nesting bound")
    if isinstance(value, dict):
        for key, item in value.items():
            require(isinstance(key, str), "resource key type")
            normalized = re.sub(r"[_-]", "", key).casefold()
            require(normalized not in PRIVATE_FIELDS, "private resource fields rejected")
            reject_private_fields(item, depth + 1)
    elif isinstance(value, list):
        for item in value:
            reject_private_fields(item, depth + 1)


def integer(value, minimum=0, maximum=2**64 - 1):
    """Accept exact bounded counters, excluding booleans and lossy numeric strings."""
    require(type(value) is int and minimum <= value <= maximum, "resource integer counter")
    return value


def number(value, maximum=1e20):
    """Accept only finite nonnegative aggregate measurements."""
    require(type(value) in (int, float) and 0 <= value <= maximum and
            math.isfinite(value), "resource numeric counter")
    return value


def availability(value):
    """Retain typed availability facts without arbitrary subprocess error text."""
    require(isinstance(value, dict) and type(value.get("available")) is bool,
            "resource availability required")
    result = {"available": value["available"]}
    if not value["available"]:
        if "error" in value:
            result["error"] = "counter unavailable"
        if "exitCode" in value:
            result["exitCode"] = integer(value["exitCode"], -(2**31), 2**32 - 1)
        if "oversized" in value:
            require(type(value["oversized"]) is bool, "resource oversized flag")
            result["oversized"] = value["oversized"]
    return result


def host_counters(value, windows=False):
    """Copy only numeric Windows or Linux aggregate CPU/load/memory counters."""
    result = availability(value)
    if not result["available"]:
        return result
    if windows:
        for key in ("cpuPercent", "totalMemoryKiB", "freeMemoryKiB"):
            if key in value:
                result[key] = integer(value[key], maximum=100 if key == "cpuPercent" else 2**64 - 1)
    else:
        if "cpuTicks" in value:
            ticks = value["cpuTicks"]
            require(isinstance(ticks, list) and 4 <= len(ticks) <= 10, "kernel CPU counter shape")
            result["cpuTicks"] = [integer(tick) for tick in ticks]
        if "loadAverage" in value:
            load = value["loadAverage"]
            require(isinstance(load, list) and len(load) == 3, "kernel load shape")
            result["loadAverage"] = [number(item) for item in load]
        if "memoryKiB" in value:
            memory = value["memoryKiB"]
            require(isinstance(memory, dict), "kernel memory shape")
            result["memoryKiB"] = {key: integer(memory[key]) for key in MEMORY_FIELDS if key in memory}
    return result


def gpu_counters(value):
    """Keep known vendor counter cells and explicit missing values, excluding GPU names."""
    result = availability(value)
    if not result["available"]:
        return result
    rows = value.get("rows")
    require(isinstance(rows, list) and len(rows) <= 32, "GPU counter row bound")
    result["rows"] = []
    for row in rows:
        require(isinstance(row, dict), "GPU counter row shape")
        cells = {}
        for key, unit in zip(GPU_FIELDS, GPU_UNITS):
            if key in row:
                cell = row[key]
                require(isinstance(cell, str) and len(cell) <= 32 and
                        (re.fullmatch(r"\d+(?:\.\d+)?(?:" + re.escape(unit) + r")?", cell) is not None or
                         cell in ("[N/A]", "N/A", "[Not Supported]")), "GPU counter cell")
                cells[key] = cell
        require(cells, "GPU counters required")
        result["rows"].append(cells)
    return result


def public_snapshot(value):
    """Construct one counter-only observation without copying arbitrary source fields."""
    require(isinstance(value, dict), "resource snapshot shape")
    require(value.get("processDetailsIncluded") is False, "local process details rejected")
    reject_private_fields(value)
    require(type(value.get("schemaVersion")) is int and value["schemaVersion"] == 1,
            "resource schema version")
    platform = value.get("platform")
    require(platform in ("Windows", "Linux"), "resource platform")
    utc = value.get("utc")
    require(isinstance(utc, str) and len(utc) <= 40 and
            re.fullmatch(r"[0-9T:+.Z-]+", utc) is not None, "resource UTC format")
    try:
        timestamp = datetime.fromisoformat(utc)
    except ValueError:
        raise ValueError("resource UTC format") from None
    require(timestamp.tzinfo is not None and timestamp.utcoffset().total_seconds() == 0,
            "resource UTC timezone")
    source = value.get("observerSourceSha256")
    require(isinstance(source, str) and re.fullmatch(r"[0-9a-f]{64}", source) is not None,
            "observer source identity")
    result = {"schemaVersion": 1, "utc": timestamp.astimezone(timezone.utc).isoformat(),
              "platform": platform, "gpu": gpu_counters(value.get("gpu")),
              "host": host_counters(value.get("host"), windows=platform == "Windows"),
              "processDetailsIncluded": False, "attribution": ATTRIBUTION,
              "collectionSeconds": number(value.get("collectionSeconds")),
              "sample": integer(value.get("sample"), maximum=MAX_SAMPLES - 1),
              "observerSourceSha256": source}
    if platform == "Windows":
        wsl = value.get("wsl")
        result["wsl"] = availability(wsl)
        if result["wsl"]["available"]:
            distributions = wsl.get("distributions")
            require(isinstance(distributions, list) and len(distributions) <= 16, "WSL counter row bound")
            result["wsl"]["distributions"] = [host_counters(row) for row in distributions]
    else:
        require("wsl" not in value, "WSL counters require Windows")
    return result


def decode_json(raw):
    """Reject duplicate keys and nonfinite constants before any counter projection."""
    def unique_pairs(pairs):
        """Prevent a second key from hiding a debug marker or counter."""
        result = {}
        for key, value in pairs:
            require(key not in result, "duplicate resource JSON key")
            result[key] = value
        return result

    def invalid_constant(_value):
        """Avoid accepting non-standard NaN/Infinity JSON values."""
        raise ValueError("nonfinite resource JSON constant")

    try:
        return json.loads(raw, object_pairs_hook=unique_pairs, parse_constant=invalid_constant)
    except (ValueError, RecursionError, UnicodeError):
        raise ValueError("invalid resource JSON") from None


def export_counters(source, destination):
    """Validate the complete bounded input before exclusively creating a public sidecar."""
    rows = []
    total_input = 0
    total_output = 0
    previous = None
    with source.open("rb") as stream:
        while True:
            raw = stream.readline(MAX_LINE_BYTES + 1)
            if not raw:
                break
            total_input += len(raw)
            require(len(raw) <= MAX_LINE_BYTES and total_input <= MAX_INPUT_BYTES, "resource input byte bound")
            require(len(rows) < MAX_SAMPLES, "resource sample bound")
            row = public_snapshot(decode_json(raw))
            if previous is not None:
                require(row["platform"] == previous["platform"] and
                        row["observerSourceSha256"] == previous["observerSourceSha256"],
                        "resource source changed")
                require(row["sample"] > previous["sample"], "resource samples not ordered")
            encoded = (json.dumps(row, allow_nan=False, separators=(",", ":")) + "\n").encode("utf-8")
            total_output += len(encoded)
            require(total_output <= MAX_OUTPUT_BYTES, "resource output byte bound")
            rows.append(encoded)
            previous = row
    require(rows, "resource samples required")
    # Exclusive creation preserves an existing report or the original source.
    with destination.open("xb") as stream:
        try:
            stream.writelines(rows)
        except BaseException:
            stream.close()
            destination.unlink()
            raise
    return len(rows), total_output


def main():
    """Export aggregates without printing source paths, vendor text or private input values."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    try:
        count, size = export_counters(args.input, args.output)
    except ValueError as error:
        parser.exit(2, "Resource counter export rejected: " + str(error) + "\n")
    except OSError:
        parser.exit(2, "Resource counter export could not read or exclusively create its files.\n")
    print(f"Exported {count} aggregate observations ({size} bytes).")


if __name__ == "__main__":
    main()
