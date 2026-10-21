"""Explicit, isolated Cargo build-mode qualification; never invokes an installer.

Run separately from cargo test to avoid recursive Cargo target-lock deadlocks.
"""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ROOT = project_root()


def main():
    target = Path(os.environ.get("CARGO_TARGET_DIR", ROOT / "target/updater-config-checks"))
    if not target.is_absolute():
        target = ROOT / target
    target.mkdir(parents=True, exist_ok=True)
    base = {k: v for k, v in os.environ.items()
            if not k.startswith(("CRAFT_UPDATE", "CRAFT_RELEASE", "CRAFT_EXPECT"))
            and k not in {"CRAFT_MANAGER_REPOSITORY", "CRAFT_BUILD_REVISION"}}
    base["CARGO_TARGET_DIR"] = str(target)
    target_os = {"win32": "windows", "darwin": "macos"}.get(sys.platform, "linux")
    cases = [("github-default", False, {}), ("library-disabled", True, {}),
             ("service-opt-in", True, {"CRAFT_UPDATE_SOURCE": "service",
                                      "CRAFT_UPDATE_CATALOG_URL": "https://updates.invalid/v2/catalog.json"})]
    with tempfile.TemporaryDirectory(prefix="build-matrix-", dir=target) as folder:
        folder = Path(folder)
        config = folder / "updater.toml"
        key = folder / "key.hex"
        key.write_text("07" * 32)
        config.write_text("repository='fork/manager'\npublic_key_file='key.hex'\n[revisions]\nwindows=2\nlinux=3\nmacos=4\n")
        cases.append(("fork-key-and-revision", False, {"CRAFT_UPDATER_CONFIG": str(config)}))
        for name, feature, additions in cases:
            env = dict(base, **additions)
            view = json.loads(subprocess.check_output(
                [sys.executable, "scripts/lib/updater_config.py", "export", "--target-os", target_os],
                cwd=ROOT, env=env, text=True))
            env.update(CRAFT_EXPECT_SOURCE=view["source"], CRAFT_EXPECT_REPOSITORY=view["repository"],
                       CRAFT_EXPECT_REVISION=str(view["revision"]), CRAFT_EXPECT_KEY=view["public_key_fingerprint"],
                       CRAFT_EXPECT_FINGERPRINT=view["runtime_config_fingerprint"])
            command = ["cargo", "test", "--locked", "--test", "updater_config"]
            if feature:
                command += ["--features", "service-updater"]
            command += ["embedded_settings_match_requested_build", "--", "--exact", "--test-threads=1"]
            result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True, text=True)
            if result.returncode or "running 1 test" not in result.stdout:
                raise RuntimeError(name + " failed\n" + result.stdout + result.stderr)
            print(name + ": passed", flush=True)
        # Same file, same key length: Cargo must notice the referenced key changed.
        key.write_text("08" * 32)
        env = dict(base, CRAFT_UPDATER_CONFIG=str(config))
        view = json.loads(subprocess.check_output([sys.executable, "scripts/lib/updater_config.py", "export", "--target-os", target_os], cwd=ROOT, env=env, text=True))
        env["CRAFT_EXPECT_KEY"] = view["public_key_fingerprint"]
        subprocess.run(["cargo", "test", "--locked", "--test", "updater_config", "embedded_settings_match_requested_build", "--", "--exact", "--test-threads=1"], cwd=ROOT, env=env, check=True, capture_output=True)
        print("referenced-key-rebuild: passed", flush=True)
    env = dict(base, CRAFT_UPDATE_SOURCE="service", CRAFT_UPDATE_CATALOG_URL="https://updates.invalid/v2/catalog.json")
    result = subprocess.run(["cargo", "check", "--locked", "--lib"], cwd=ROOT, env=env, capture_output=True, text=True)
    if result.returncode == 0 or "--features service-updater" not in result.stderr:
        raise RuntimeError("Service without its feature did not fail safely")
    print("service-without-feature: rejected as expected", flush=True)
    # This must be a dependency-tree assertion, not a source-string assertion.
    tree = subprocess.check_output(["cargo", "tree", "--locked", "-p", "craft-apps-manager", "-e", "normal"], cwd=ROOT, env=base, text=True)
    if "craft-updater-service" in tree:
        raise RuntimeError("Default source build pulls in service runtime")
    print("default-service-dependency: absent", flush=True)


if __name__ == "__main__":
    main()
