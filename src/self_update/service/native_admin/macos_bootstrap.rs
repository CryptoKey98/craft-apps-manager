//! Fixed bootstrap text. All caller-controlled values are quoted arguments.
use anyhow::{ensure, Result};
use std::path::Path;

fn quote(value: &str) -> Result<String> {
    ensure!(
        !value.chars().any(char::is_control),
        "Control character in approval argument"
    );
    Ok(format!("'{}'", value.replace('\'', "'\\''")))
}

pub(super) fn script(
    helper: &Path,
    manifest: &Path,
    manifest_sha256: &str,
    helper_sha256: &str,
    caller_uid: u32,
) -> Result<String> {
    let helper = helper
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid helper path"))?;
    let manifest = manifest
        .to_str()
        .ok_or_else(|| anyhow::anyhow!("Invalid manifest path"))?;
    ensure!(
        helper.starts_with('/') && manifest.starts_with('/'),
        "Approval paths must be absolute"
    );
    ensure!(
        caller_uid > 0 && caller_uid < u32::MAX,
        "Invalid caller UID"
    );
    for hash in [manifest_sha256, helper_sha256] {
        ensure!(
            hash.len() == 64
                && hash
                    .bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)),
            "Invalid approval checksum"
        );
    }
    let command = format!(
        r#"set -eu
umask 077
work=$(/usr/bin/mktemp -d /private/tmp/craft-native-approved-XXXXXXXX)
trap '/bin/rm -rf "$work"' EXIT HUP INT TERM
/bin/chmod -N "$work"
/bin/chmod 700 "$work"
test ! -e "$work/admin-helper" && test ! -L "$work/admin-helper"
test ! -e "$work/approved.json" && test ! -L "$work/approved.json"
/bin/cp -X {helper} "$work/admin-helper"
/bin/cp -X {manifest} "$work/approved.json"
/bin/chmod 700 "$work/admin-helper"
/bin/chmod 600 "$work/approved.json"
test "$(/usr/bin/shasum -a 256 "$work/admin-helper" | /usr/bin/awk '{{print $1}}')" = '{helper_sha256}'
test "$(/usr/bin/shasum -a 256 "$work/approved.json" | /usr/bin/awk '{{print $1}}')" = '{manifest_sha256}'
(
trap '/bin/rm -rf "$work"' EXIT HUP INT TERM
/usr/bin/env -i PATH=/usr/bin:/bin:/usr/sbin:/sbin HOME=/var/root LANG=C "$work/admin-helper" --native-bridge-admin "$work/approved.json" '{manifest_sha256}' --macos-approval-uid '{caller_uid}'
) </dev/null >/dev/null 2>&1 &
pid=$!
trap - EXIT HUP INT TERM
printf '%s' "$pid""#,
        helper = quote(helper)?,
        manifest = quote(manifest)?
    );
    // AppleScript and shell quoting are separate boundaries.
    let escaped = command
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    Ok(format!(
        "do shell script \"{escaped}\" with administrator privileges"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[cfg(target_os = "macos")]
    fn bootstrap_compiles_without_running_an_administrator_prompt() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-bootstrap-parse-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let text = script(
            Path::new("/private/tmp/a ' \" ` $ (b)/helper"),
            Path::new("/private/tmp/approval.json"),
            &"a".repeat(64),
            &"b".repeat(64),
            501,
        )
        .unwrap();
        let source = root.join("bootstrap.applescript");
        std::fs::write(&source, text).unwrap();
        let output = std::process::Command::new("/usr/bin/osacompile")
            .arg("-o")
            .arg(root.join("bootstrap.scpt"))
            .arg(&source)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn bootstrap_accepts_literal_paths_and_rejects_malformed_authority() {
        let path = Path::new("/private/tmp/a ' \" ` $ (b)/helper");
        assert!(script(path, path, &"a".repeat(64), &"b".repeat(64), 501).is_ok());
        for hash in ["abc", &"g".repeat(64), &"a\n".repeat(32)] {
            assert!(script(path, path, hash, &"b".repeat(64), 501).is_err());
        }
        assert!(script(path, path, &"a".repeat(64), &"b".repeat(64), 0).is_err());
        assert!(script(
            Path::new("/tmp/\nhelper"),
            path,
            &"a".repeat(64),
            &"b".repeat(64),
            501
        )
        .is_err());
        assert!(script(
            Path::new("relative"),
            path,
            &"a".repeat(64),
            &"b".repeat(64),
            501
        )
        .is_err());
    }
    #[test]
    fn quoted_shell_arguments_preserve_metacharacters() {
        // Exercise the shell boundary rather than assert renderer source text.
        let shell = if cfg!(windows) {
            "C:/Program Files/Git/bin/bash.exe"
        } else {
            "/bin/sh"
        };
        for input in ["normal", "a b", "'\"`$HOME$(echo BAD);\\", "---"] {
            let output = std::process::Command::new(shell)
                .arg("-c")
                .arg(format!("printf '%s' {}", quote(input).unwrap()))
                .output()
                .unwrap();
            assert!(output.status.success());
            assert_eq!(output.stdout, input.as_bytes());
        }
        assert!(quote("\n").is_err());
    }
}
