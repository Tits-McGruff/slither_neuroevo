"""Resource attribution must include running WSL without changing other workloads."""
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location(
    "system_resource_observer", Path(__file__).with_name("system-resource-observer.py"))
OBSERVER = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(OBSERVER)


class ResourceObservationTests(unittest.TestCase):
    """Verify inventory and unavailable-counter semantics without external processes."""

    def test_wsl_utf16_inventory(self):
        self.assertEqual(OBSERVER.decode_output("Ubuntu\r\n".encode("utf-16le")), "Ubuntu")
        self.assertEqual(OBSERVER.decode_output(b"Ubuntu\n"), "Ubuntu")

    def test_only_running_distributions_are_queried(self):
        calls = []
        def output(command):
            calls.append(command)
            return {"available": True, "text": "Ubuntu\n" if len(calls) == 1 else "resource counters"}
        with patch.object(OBSERVER, "command_output", side_effect=output):
            result = OBSERVER.running_wsl_resources()
        self.assertEqual(calls[0], ["wsl.exe", "--list", "--running", "--quiet"])
        self.assertEqual(calls[1][0:4], ["wsl.exe", "--distribution", "Ubuntu", "--exec"])
        self.assertEqual(len(calls), 2)
        self.assertEqual(result["distributions"][0]["distribution"], "Ubuntu")
        self.assertNotIn("cmdline", calls[1][-1])
        self.assertNotIn("args", calls[1][-1])

    def test_missing_wsl_is_not_an_empty_idle_host(self):
        with patch.object(OBSERVER, "command_output", return_value={"available": False}):
            self.assertEqual(OBSERVER.running_wsl_resources(), {"available": False})

    def test_gpu_unknown_memory_is_preserved_and_paths_removed(self):
        text = "pid, process_name, used_gpu_memory [MiB]\n12, C:\\Tools\\python.exe, [N/A]\n"
        with patch.object(OBSERVER, "command_output", return_value={"available": True, "text": text}):
            result = OBSERVER.csv_observation(["nvidia-smi"], process_names=True)
        self.assertEqual(result["rows"][0]["process_name"], "python.exe")
        self.assertEqual(result["rows"][0]["used_gpu_memory [MiB]"], "[N/A]")

    def test_command_timeout_is_explicit(self):
        import subprocess
        with patch.object(OBSERVER.subprocess, "run",
                          side_effect=subprocess.TimeoutExpired("counter", 8)):
            result = OBSERVER.command_output(["counter"])
        self.assertFalse(result["available"])
        self.assertEqual(result["error"], "TimeoutExpired")


if __name__ == "__main__":
    unittest.main()
