"""Resolve external historical receipts; required audits never silently skip missing inputs."""
import json
import os
from pathlib import Path


def required_root():
    """Use the checksum-addressed cache populated by the maintained PyRL evidence store."""
    repo = Path(__file__).resolve().parents[2]
    catalog = json.loads((repo / "docs/performance/catalog.json").read_bytes())
    root = Path(os.environ.get("QUALIFICATION_EVIDENCE_ROOT", str(
        repo / ".cache/qualification-evidence" / catalog["artifact"]["sha256"])))
    if not (root / "verified.json").is_file():
        raise RuntimeError("Required historical evidence unavailable. Run python "
                           "PATH_TO_PYRL/pyrl_trainer/evidence_store.py fetch "
                           "--catalog docs/performance/catalog.json "
                           "--cache .cache/qualification-evidence; missing assets fail qualification.")
    return root
