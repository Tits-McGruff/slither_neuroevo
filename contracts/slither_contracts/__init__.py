"""Generated structural contracts; validation and execution remain in each consumer."""
import hashlib
from importlib.resources import files
import json

CONSTANTS = json.loads(files(__package__).joinpath("constants.json").read_text(encoding="utf-8"))
# Exact installed definitions are part of trainer restart compatibility.
IDENTITY = hashlib.sha256(b"".join(
    files(__package__).joinpath(name).read_bytes()
    for name in ("__init__.py", "models.py", "constants.json", "contracts.schema.json")
)).hexdigest()
