"""Resolve this checkout independently of the caller's working directory."""
from pathlib import Path


def project_root() -> Path:
    root = Path(__file__).resolve().parents[2]
    if not (root / "Cargo.toml").is_file() or not (root / "scripts").is_dir():
        raise RuntimeError("Expected the Craft Apps Manager project beside scripts/lib")
    return root
