"""Regenerate native Rust contracts into JSON Schema, Python and browser types."""
from pathlib import Path
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]
subprocess.run([
    "cargo", "run", "--manifest-path", "native/Cargo.toml", "--features",
    "contract-schema", "--bin", "export-contracts", "--", "contracts/slither_contracts",
], cwd=ROOT, check=True)
subprocess.run([
    sys.executable, "-m", "datamodel_code_generator", "--input",
    "contracts/slither_contracts/contracts.schema.json", "--output",
    "contracts/slither_contracts/models.py", "--input-file-type", "jsonschema",
    "--output-model-type", "typing.TypedDict", "--target-python-version", "3.10",
    "--disable-timestamp", "--formatters", "black", "isort",
], cwd=ROOT, check=True)
subprocess.run(["node", "--input-type=module", "-e", """
import fs from 'node:fs';
import {compile} from 'json-schema-to-typescript';
const schema = JSON.parse(fs.readFileSync('contracts/slither_contracts/contracts.schema.json', 'utf8'));
// Browser consumes graph presentation and routed dataset metadata, not trainer internals.
delete schema.properties.training;
delete schema.properties.brain;
schema.required = schema.required.filter(name => name !== 'training' && name !== 'brain');
const constants = JSON.parse(fs.readFileSync('contracts/slither_contracts/constants.json', 'utf8'));
const domain = '\\nexport const SENSOR_HASH_DOMAIN = ' + JSON.stringify(constants.sensorHashDomain) + ';\\n';
fs.writeFileSync('contracts/types.ts', await compile(schema, 'BrowserContracts') + domain);
"""], cwd=ROOT, check=True)
