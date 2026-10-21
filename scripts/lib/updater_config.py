"""Resolved source-build updater configuration. No signing, uploads or deployment."""

# Resolve sibling helpers from this file, never from the invoking directory.
import sys as _sys
from pathlib import Path as _Path
_sys.path.insert(0, str(_Path(__file__).resolve().parents[1]))
from lib.repository import project_root
import argparse
import json
import os
from pathlib import Path
import struct
import subprocess
import sys

ROOT = project_root()


def build_environment(config_path=None):
    env = dict(os.environ)
    if config_path is not None:
        env["CRAFT_UPDATER_CONFIG"] = str(config_path)
    for alias, canonical in [("CRAFT_RELEASE_CATALOG_URL","CRAFT_UPDATE_CATALOG_URL"),("CRAFT_RELEASE_CATALOG_MIRROR","CRAFT_UPDATE_CATALOG_MIRROR"),("CRAFT_RELEASE_CHECK_URL","CRAFT_UPDATE_CHECK_URL")]:
        if env.get(alias) and canonical not in env:
            env[canonical] = env[alias]
    return env


def load_view(view, target_os, config=None, overrides=None):
    command = ["cargo", "run", "--locked", "--quiet", "-p", "craft-updater-config",
               "--bin", "updater-config-tool", "--", "export", "--project-root", str(ROOT),
               "--view", view, "--target-os", target_os]
    if os.environ.get("CRAFT_UPDATER_CONFIG_TOOL"):
        command = [os.environ["CRAFT_UPDATER_CONFIG_TOOL"]] + command[command.index("--") + 1:]
    if config is not None:
        command += ["--config", str(config)]
    for name, value in (overrides or {}).items():
        if name not in {"repository", "channel", "revision", "catalog-url", "catalog-mirror", "check-url"}:
            raise ValueError("Unsupported configuration override")
        command += ["--" + name, str(value)]
    env = build_environment()
    # Exporting config must not build the GUI or write to a user's release tree.
    env.setdefault("CARGO_TARGET_DIR", str(ROOT / "target/updater-config-checks"))
    value = json.loads(subprocess.check_output(command, cwd=ROOT, env=env, text=True))
    if value.get("schema_version") != 1:
        raise ValueError("Unsupported effective configuration schema")
    return value


def build_command(config, target):
    features = config["required_features"]
    if features != (["service-updater"] if config["source"] == "service" else []):
        raise ValueError("Invalid updater source/feature combination")
    args = ["cargo", "build", "--release", "--locked", "--bin", "craft-apps-manager"]
    if target:
        args += ["--target", target]
    if features:
        args += ["--features", ",".join(features)]
    return args


def verify_identity(identity, config, version):
    expected = {"version": version, "revision": config["revision"],
                "update_source": config["source"], "update_repository": config["repository"],
                "update_channel": config["channel"],
                "update_key_fingerprint": config["public_key_fingerprint"],
                "update_config_fingerprint": config["runtime_config_fingerprint"]}
    if any(identity.get(k) != v for k, v in expected.items()):
        raise ValueError("Executable updater identity/configuration does not match the package")


def verify_macos_bytes(data, config, version, require_universal=False):
    """Read bounded Mach-O identity sections; never execute a foreign CPU slice."""
    def fail():
        raise ValueError("Missing, malformed or mismatched Mach-O updater identity")
    if not 32 <= len(data) <= 512 * 1024 * 1024:
        fail()
    if data[:4] in (b"\xca\xfe\xba\xbe", b"\xca\xfe\xba\xbf"):
        wide = data[:4] == b"\xca\xfe\xba\xbf"
        count = struct.unpack_from(">I", data, 4)[0]
        width = 32 if wide else 20
        if not 1 <= count <= 8 or 8 + count * width > len(data):
            fail()
        slices=[]
        for i in range(count):
            entry=8+i*width
            cpu=struct.unpack_from(">I",data,entry)[0]
            offset,size=struct.unpack_from(">QQ" if wide else ">II",data,entry+8)
            if offset < 8 + count*width or size < 32 or offset+size > len(data):
                fail()
            if any(offset < other+length and other < offset+size for _,other,length in slices):
                fail()
            slices.append((cpu,offset,size))
    else:
        slices=[(None,0,len(data))]
    targets={0x01000007:"x86_64-apple-darwin",0x0100000c:"aarch64-apple-darwin"}
    seen=set()
    for outer_cpu,start,size in slices:
        block=memoryview(data)[start:start+size]
        if bytes(block[:4]) != b"\xcf\xfa\xed\xfe":
            fail()
        cpu,=struct.unpack_from("<I",block,4)
        if cpu not in targets or cpu in seen or outer_cpu not in (None,cpu):
            fail()
        seen.add(cpu)
        ncmds,cmdsize=struct.unpack_from("<II",block,16)
        if ncmds > 4096 or cmdsize > size-32:
            fail()
        cursor=32
        found=[]
        for _ in range(ncmds):
            if cursor+8 > 32+cmdsize:
                fail()
            cmd,length=struct.unpack_from("<II",block,cursor)
            if length < 8 or cursor+length > 32+cmdsize:
                fail()
            if cmd == 0x19:
                if length < 72:
                    fail()
                nsects=struct.unpack_from("<I",block,cursor+64)[0]
                if 72+nsects*80 > length:
                    fail()
                for section in range(nsects):
                    pos=cursor+72+section*80
                    name=bytes(block[pos:pos+16]).rstrip(b"\0")
                    segment=bytes(block[pos+16:pos+32]).rstrip(b"\0")
                    if (segment,name)==(b"__TEXT",b"__craft_update"):
                        amount=struct.unpack_from("<Q",block,pos+40)[0]
                        offset=struct.unpack_from("<I",block,pos+48)[0]
                        if not 1 <= amount <= 4096 or offset < 32+cmdsize or offset+amount > size:
                            fail()
                        try:
                            found.append(json.loads(bytes(block[offset:offset+amount])))
                        except (ValueError,UnicodeError):
                            fail()
            cursor+=length
        if cursor != 32+cmdsize or len(found) != 1 or found[0].get("target") != targets[cpu]:
            fail()
        verify_identity(found[0],config,version)
    if require_universal and seen != set(targets):
        fail()


def renewal_due(expires, now, lead_days):
    if type(expires) is not int or type(now) is not int or not 1 <= lead_days <= 29:
        raise ValueError("Invalid renewal deadline")
    return expires - now <= lead_days * 86400


def publisher_config(path=None):
    if path is not None and Path(path).suffix == ".json":
        # Explicit old fixtures remain independent; never merge in the root TOML.
        print("Legacy publisher JSON configuration is deprecated; use updater.toml", file=sys.stderr)
        return json.loads(Path(path).read_text(encoding="utf-8"))
    value = load_view("publisher", "linux", path)
    return dict(value["publishing"], repository=value["repository"], channel=value["channel"],
                catalog_url=value["service"]["catalog_url"], mirror_url=value["service"]["mirror_url"],
                profiles=value["profiles_path"], next_catalog=value["next_catalog_path"],
                public_key_path=value["public_key_path"], public_key_hex=value["public_key_hex"])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("action", choices=["export", "build", "verify", "worker", "revision", "format"])
    parser.add_argument("--target-os", required=True, choices=["windows", "linux", "macos"])
    parser.add_argument("--view", default="build", choices=["build", "packaging", "publisher", "worker"])
    parser.add_argument("--config", type=Path)
    parser.add_argument("--target")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--identity-file", type=Path)
    parser.add_argument("--require-universal", action="store_true")
    parser.add_argument("--package-format")
    args = parser.parse_args()
    config = load_view("worker" if args.action == "worker" else args.view, args.target_os, args.config)
    if args.action == "revision":
        print(config["revision"])
        return
    if args.action == "format":
        if args.package_format not in {"zip","tar.gz","msi","deb","rpm","arch","dmg"}:
            parser.error("format requires a supported --package-format")
        print("true" if args.package_format in config["packages"][args.target_os] else "false")
        return
    if args.action == "build":
        env = build_environment(config["config_path"] if args.config else None)
        subprocess.run(build_command(config, args.target), cwd=ROOT, env=env, check=True)
        return
    if args.action == "verify":
        if args.identity_file:
            identity = json.loads(args.identity_file.read_text(encoding="utf-8-sig"))
        elif args.binary and args.target_os == "macos":
            import re
            version = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
            if args.binary.stat().st_size > 512 * 1024 * 1024:
                raise ValueError("Oversized Mach-O executable")
            verify_macos_bytes(args.binary.read_bytes(), config, version, args.require_universal)
            return
        elif args.binary:
            if args.require_universal:
                parser.error("--require-universal requires a macOS binary")
            flags = {"creationflags": subprocess.CREATE_NO_WINDOW} if sys.platform == "win32" else {}
            identity = json.loads(subprocess.check_output([str(args.binary.resolve()), "--build-identity"], text=True, timeout=20, **flags))
        else:
            parser.error("verify requires --binary or --identity-file")
        # Cargo remains the only source of application version.
        import re
        version = re.search(r'^version\s*=\s*"([^"]+)"', (ROOT / "Cargo.toml").read_text(), re.M)[1]
        verify_identity(identity, config, version)
        return
    value = config["worker_config"] if args.action == "worker" else config
    text = json.dumps(value, indent=2) + "\n"
    if args.out:
        generated = ROOT / "update-service/wrangler.generated.json"
        # Only the fixed, ignored generated Worker file can be refreshed.
        if args.action == "worker" and args.out.absolute() == generated.absolute():
            if args.out.is_symlink():
                raise ValueError("Linked generated Worker config")
            args.out.write_text(text, encoding="utf-8")
        else:
            with args.out.open("x", encoding="utf-8") as output:
                output.write(text)
    else:
        print(text, end="")


if __name__ == "__main__":
    main()
