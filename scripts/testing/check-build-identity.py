"""Reject packaging a stale executable or a build with the wrong revision."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import json
import subprocess

parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--binary", required=True)
parser.add_argument("--version", required=True)
parser.add_argument("--revision", type=int, required=True)
args = parser.parse_args()
identity = json.loads(subprocess.check_output([args.binary, "--build-identity"], timeout=15, text=True))
if identity.get("version") != args.version or identity.get("revision") != args.revision:
    raise SystemExit("Executable version/revision does not match the package being prepared")
