use crate::{
    files,
    jobs::Job,
    model::{Asset, Paths, Release},
    network::Network,
    platform,
};
use anyhow::{bail, Context, Result};
use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
pub fn find_in(folder: &Path, exe: &str) -> Option<PathBuf> {
    if !folder.exists() {
        return None;
    }
    walkdir::WalkDir::new(folder)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
        .find(|e| {
            e.file_type().is_file() && e.file_name().to_string_lossy().eq_ignore_ascii_case(exe)
        })
        .map(|e| e.into_path())
}
pub fn system(exe: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|p| {
        std::env::split_paths(&p)
            .map(|dir| dir.join(exe))
            .find(|p| p.is_file())
    })
}
pub fn find(paths: &Paths, folder: &str, exe: &str) -> Option<PathBuf> {
    find_in(&paths.tools.join(folder), exe).or_else(|| system(exe))
}
pub fn cargo(paths: &Paths) -> Option<PathBuf> {
    let local = paths.tools.join("cargo/bin/cargo.exe");
    if local.exists() {
        Some(local)
    } else {
        system("cargo.exe")
    }
}
pub fn seven(paths: &Paths) -> Option<PathBuf> {
    let native = paths
        .tools
        .join(format!("7zip/{}/7za.exe", crate::model::MANAGER_ARCH));
    if native.exists() {
        return Some(native);
    }
    paths
        .tools
        .join("7zip/7za.exe")
        .is_file()
        .then(|| paths.tools.join("7zip/7za.exe"))
        .or_else(|| {
            let exe = std::env::current_exe().ok()?;
            let bundled = exe
                .parent()?
                .join(format!("tools/7zip/{}/7za.exe", crate::model::MANAGER_ARCH));
            bundled.is_file().then_some(bundled)
        })
        .or_else(|| {
            let p = PathBuf::from("C:/Program Files/7-Zip/7z.exe");
            p.exists().then_some(p)
        })
        .or_else(|| system("7z.exe"))
}
pub fn visual_cpp() -> Result<Option<PathBuf>> {
    let exe = Path::new("C:/Program Files (x86)/Microsoft Visual Studio/Installer/vswhere.exe");
    if !exe.exists() {
        return Ok(None);
    }
    let out = platform::output(Command::new(exe).args([
        "-latest",
        "-products",
        "*",
        "-requires",
        "Microsoft.VisualStudio.Component.VC.Tools.x86.x64",
        "-property",
        "installationPath",
    ]))?;
    if !out.status.success() {
        bail!("vswhere failed")
    };
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| PathBuf::from(s).join("Common7/Tools/VsDevCmd.bat"))
        .find(|p| p.exists()))
}
/// Older Windows build tools still impose MAX_PATH despite long-path support.
/// Use filesystem-provided aliases without relocating or linking user folders.
pub fn compiler_path(path: &Path) -> Result<PathBuf> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows::{core::PCWSTR, Win32::Storage::FileSystem::GetShortPathNameW};
    let absolute = std::path::absolute(path)?;
    let wide: Vec<u16> = absolute.as_os_str().encode_wide().chain([0]).collect();
    let mut buffer = vec![0u16; 32768];
    let count = unsafe { GetShortPathNameW(PCWSTR(wide.as_ptr()), Some(&mut buffer)) } as usize;
    let result = if count > 0 && count < buffer.len() {
        PathBuf::from(std::ffi::OsString::from_wide(&buffer[..count]))
    } else {
        absolute
    };
    anyhow::ensure!(result.as_os_str().encode_wide().count() <= 140,
        "Windows build path is too long and a short folder alias is unavailable: {}. Choose a shorter library or build-tools folder in Settings > Folders (for example C:/CraftBuildTools).", path.display());
    Ok(result)
}
/// Rustup canonicalizes its home, so explicitly shorten rustc's sysroot too.
pub fn configure_rust_paths(
    paths: &Paths,
    project: &Path,
    env: &mut BTreeMap<String, String>,
) -> Result<()> {
    let rustc = paths.tools.join("cargo/bin/rustc.exe");
    let rustc = if rustc.is_file() {
        rustc
    } else {
        system("rustc.exe").context("Rust compiler is missing")?
    };
    let out = platform::output(
        Command::new(compiler_path(&rustc)?)
            .args(["--print", "sysroot"])
            .current_dir(compiler_path(project)?)
            .envs(&*env),
    )?;
    anyhow::ensure!(
        out.status.success(),
        "Could not locate the Rust system libraries: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let sysroot = PathBuf::from(String::from_utf8(out.stdout)?.trim());
    let alias = compiler_path(&sysroot)?;
    let mut flags = env
        .get("CARGO_ENCODED_RUSTFLAGS")
        .cloned()
        .unwrap_or_else(|| {
            env.get("RUSTFLAGS")
                .map(|flags| flags.split_whitespace().collect::<Vec<_>>().join("\x1f"))
                .unwrap_or_default()
        });
    if !flags.is_empty() {
        flags.push('\x1f');
    }
    flags.push_str("--sysroot");
    flags.push('\x1f');
    flags.push_str(&alias.display().to_string());
    env.insert("CARGO_ENCODED_RUSTFLAGS".into(), flags);
    Ok(())
}
pub fn environment(paths: &Paths) -> Result<BTreeMap<String, String>> {
    use std::os::windows::process::CommandExt;
    let vs = visual_cpp()?
        .context("Microsoft C++ build tools are missing. Click Set up build tools.")?;
    let out = platform::output(Command::new("cmd.exe").raw_arg(format!(
        "/d /s /c \"call \"{}\" -no_logo -arch=x64 -host_arch=x64 >nul && set\"",
        vs.display()
    )))?;
    if !out.status.success() {
        bail!(
            "Could not initialize Microsoft C++ tools: {} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut env = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some((k, v)) = line.split_once('=') {
            if !k.is_empty() {
                env.insert(k.to_uppercase(), v.to_string());
            }
        }
    }
    let mut bins = Vec::new();
    if paths.tools.join("cargo/bin/cargo.exe").exists() {
        env.insert(
            "CARGO_HOME".into(),
            compiler_path(&paths.tools.join("cargo"))?
                .display()
                .to_string(),
        );
        env.insert(
            "RUSTUP_HOME".into(),
            compiler_path(&paths.tools.join("rustup"))?
                .display()
                .to_string(),
        );
        bins.push(paths.tools.join("cargo/bin"));
    }
    for (folder, exe) in [
        ("cmake", "cmake.exe"),
        ("perl", "perl.exe"),
        ("nasm", "nasm.exe"),
        ("llvm", "clang.exe"),
        ("node", "node.exe"),
        ("git", "git.exe"),
    ] {
        if let Some(tool) = find(paths, folder, exe) {
            bins.push(tool.parent().unwrap().to_path_buf());
            if folder == "nasm" {
                env.insert("ASM_NASM".into(), tool.display().to_string());
            }
        }
    }
    if let Some(lib) = find_in(&paths.tools.join("llvm"), "libclang.dll") {
        env.insert(
            "LIBCLANG_PATH".into(),
            lib.parent().unwrap().display().to_string(),
        );
    }
    let mut path = bins
        .iter()
        .map(|p| p.display().to_string())
        .collect::<Vec<_>>()
        .join(";");
    path.push(';');
    path.push_str(env.get("PATH").map(String::as_str).unwrap_or(""));
    env.insert("PATH".into(), path);
    for (k, v) in [
        ("NO_COLOR", "1"),
        ("FORCE_COLOR", "0"),
        ("TERM", "dumb"),
        ("CARGO_TERM_COLOR", "never"),
    ] {
        env.insert(k.into(), v.into());
    }
    Ok(env)
}
pub fn preflight(paths: &Paths, app: &str) -> Result<()> {
    require_build_host()?;
    cargo(paths).context("Rust is missing. Click Set up build tools first.")?;
    visual_cpp()?.context("Microsoft C++ tools are missing. Click Set up build tools first.")?;
    if app == "artcraftx" {
        let mut missing = Vec::new();
        for (folder, exe) in [
            ("cmake", "cmake.exe"),
            ("perl", "perl.exe"),
            ("nasm", "nasm.exe"),
            ("llvm", "clang.exe"),
            ("node", "node.exe"),
            ("git", "git.exe"),
        ] {
            if find(paths, folder, exe).is_none() {
                missing.push(folder)
            }
        }
        if find_in(&paths.tools.join("llvm"), "libclang.dll").is_none() {
            missing.push("libclang");
        }
        if !missing.is_empty() {
            bail!(
                "Missing ArtCraft X tools: {}. Select ArtCraft X and click Set up build tools.",
                missing.join(", ")
            );
        }
        let node = find(paths, "node", "node.exe").unwrap();
        let out = platform::output(Command::new(&node).arg("--version"))?;
        let version = String::from_utf8_lossy(&out.stdout)
            .trim()
            .trim_start_matches('v')
            .split('.')
            .next()
            .unwrap_or("0")
            .parse::<u32>()?;
        if version < 20
            || !node
                .parent()
                .unwrap()
                .join("node_modules/npm/bin/npm-cli.js")
                .exists()
        {
            bail!("ArtCraft X needs Node.js 20+ with npm. Click Set up build tools.");
        }
    }
    Ok(())
}
fn asset(network: &Network, repo: &str, pattern: &str) -> Result<Asset> {
    let release: Release = network.json(&format!(
        "https://api.github.com/repos/{repo}/releases/latest"
    ))?;
    let re = regex::Regex::new(pattern)?;
    let a = release
        .assets
        .into_iter()
        .find(|a| re.is_match(&a.name))
        .context(format!("No matching tool download for {repo}"))?;
    if !a
        .browser_download_url
        .starts_with(&format!("https://github.com/{repo}/releases/download/"))
    {
        bail!("Unexpected tool download URL")
    };
    files::safe_relative(&a.name)?;
    Ok(a)
}
fn require_build_host() -> Result<()> {
    if cfg!(target_arch = "x86") && std::env::var_os("PROCESSOR_ARCHITEW6432").is_none() {
        bail!("Source builds currently require 64-bit Windows. App and source updates remain available on 32-bit Windows.");
    }
    Ok(())
}
pub fn setup(paths: &Paths, app: &str, job: &Job) -> Result<()> {
    crate::model::valid_app(app)?;
    require_build_host()?;
    let _lock = platform::Lock::take("Local\\CraftAppsSourceBuilder")?;
    let network = Network::new(&paths.root)?;
    let cache = paths.at("runtime/downloads/tools");
    fs::create_dir_all(&cache)?;
    fs::create_dir_all(&paths.tools)?;
    if visual_cpp()?.is_none() {
        install_cpp(&network, &cache, job)?;
    }
    let mut env = environment(paths)?;
    if cargo(paths).is_none() {
        job.stage("Setting up tools", None, "Installing local Rust toolchain");
        let url = "https://static.rust-lang.org/rustup/dist/x86_64-pc-windows-msvc/rustup-init.exe";
        let exe = cache.join("rustup-init.exe");
        network.download(url, &exe, job)?;
        let sums = network.text(&format!("{url}.sha256"))?;
        let expected = sums
            .split_whitespace()
            .next()
            .context("Rust checksum missing")?;
        if !files::hash(&exe)?.eq_ignore_ascii_case(expected) {
            bail!("Rust installer checksum mismatch")
        }
        env.insert(
            "CARGO_HOME".into(),
            compiler_path(&paths.tools.join("cargo"))?
                .display()
                .to_string(),
        );
        env.insert(
            "RUSTUP_HOME".into(),
            compiler_path(&paths.tools.join("rustup"))?
                .display()
                .to_string(),
        );
        job.run(
            Command::new(&exe)
                .args([
                    "-y",
                    "--no-modify-path",
                    "--profile",
                    "minimal",
                    "--default-toolchain",
                    "stable",
                    "--default-host",
                    "x86_64-pc-windows-msvc",
                ])
                .envs(&env),
            false,
        )?;
        fs::remove_file(exe)?;
    } else {
        job.log("Rust is already available; reusing it.");
    }
    if app == "artcraftx" {
        native(paths, &network, &cache, job)?;
    }
    preflight(paths, app)?;
    let env = environment(paths)?;
    job.run(
        Command::new(cargo(paths).unwrap())
            .arg("--version")
            .envs(&env),
        false,
    )?;
    job.log("Build tools are ready.");
    Ok(())
}
fn native(paths: &Paths, network: &Network, cache: &Path, job: &Job) -> Result<()> {
    let seven = ensure_seven(paths, network, cache, job)?;
    for (repo, pattern, folder, exe) in [
        (
            "Kitware/CMake",
            r"-windows-x86_64\.zip$",
            "cmake",
            "cmake.exe",
        ),
        (
            "StrawberryPerl/Perl-Dist-Strawberry",
            r"-64bit-portable\.zip$",
            "perl",
            "perl.exe",
        ),
        (
            "llvm/llvm-project",
            r"^clang\+llvm-.*-x86_64-pc-windows-msvc\.tar\.xz$",
            "llvm",
            "clang.exe",
        ),
    ] {
        if find(paths, folder, exe).is_some()
            && (folder != "llvm" || find_in(&paths.tools.join(folder), "libclang.dll").is_some())
        {
            job.log(&format!("{folder} is already available"));
            continue;
        }
        let a = asset(network, repo, pattern)?;
        let archive = cache.join(&a.name);
        network.asset(&a, &archive, job)?;
        let dest = paths.tools.join(folder);
        fs::create_dir_all(&dest)?;
        if a.name.ends_with(".zip") {
            let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
            files::extract_zip(&archive, &stage, job)?;
            merge_directory(&stage, &dest)?;
            files::remove_managed(&stage, cache)?;
        } else {
            job.run(
                Command::new(&seven)
                    .arg("x")
                    .arg(&archive)
                    .arg(format!("-o{}", dest.display()))
                    .arg("-y"),
                false,
            )?;
            let tar = fs::read_dir(&dest)?
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .find(|p| p.extension().is_some_and(|e| e == "tar"))
                .context("LLVM intermediate TAR missing")?;
            job.run(
                Command::new(&seven)
                    .arg("x")
                    .arg(&tar)
                    .arg(format!("-o{}", dest.display()))
                    .arg("-y"),
                false,
            )?;
            fs::remove_file(tar)?;
        }
        if find_in(&dest, exe).is_none() {
            bail!("{exe} was not installed")
        };
        files::write_json(
            &dest.join("tool-info.json"),
            &serde_json::json!({"repository":repo,"asset":a.name,"sha256":a.digest,"installedAt":chrono::Utc::now().to_rfc3339()}),
        )?;
        fs::remove_file(archive)?;
    }
    if find(paths, "nasm", "nasm.exe").is_none() {
        let archive = cache.join("nasm-3.02-win64.zip");
        network.download(
            "https://www.nasm.us/pub/nasm/releasebuilds/3.02/win64/nasm-3.02-win64.zip",
            &archive,
            job,
        )?;
        let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
        files::extract_zip(&archive, &stage, job)?;
        merge_directory(&stage, &paths.tools.join("nasm"))?;
        files::remove_managed(&stage, cache)?;
        fs::remove_file(archive)?;
    }
    let node = find(paths, "node", "node.exe");
    let node_ok = node.as_ref().is_some_and(|p| {
        platform::output(Command::new(p).arg("--version"))
            .ok()
            .is_some_and(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .trim_start_matches('v')
                    .split('.')
                    .next()
                    .and_then(|v| v.parse::<u32>().ok())
                    .is_some_and(|v| v >= 20)
            })
            && p.parent()
                .unwrap()
                .join("node_modules/npm/bin/npm-cli.js")
                .exists()
    });
    if !node_ok {
        let releases: Vec<serde_json::Value> =
            network.json("https://nodejs.org/dist/index.json")?;
        let r = releases
            .iter()
            .find(|r| {
                r["lts"].as_str().is_some()
                    && r["files"]
                        .as_array()
                        .is_some_and(|f| f.iter().any(|s| s == "win-x64-zip"))
            })
            .context("Node.js LTS not found")?;
        let version = r["version"].as_str().unwrap();
        let name = format!("node-{version}-win-x64.zip");
        let base = format!("https://nodejs.org/dist/{version}/");
        let sums = network.text(&format!("{base}SHASUMS256.txt"))?;
        let hash = sums
            .lines()
            .find_map(|l| {
                let mut p = l.split_whitespace();
                let h = p.next()?;
                (p.next()? == name).then_some(h)
            })
            .context("Node checksum missing")?;
        let a = Asset {
            name: name.clone(),
            size: 0,
            browser_download_url: format!("{base}{name}"),
            digest: Some(format!("sha256:{hash}")),
        };
        let archive = cache.join(&name);
        network.download(&a.browser_download_url, &archive, job)?;
        if files::hash(&archive)? != hash {
            bail!("Node checksum mismatch")
        };
        let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
        files::extract_zip(&archive, &stage, job)?;
        merge_directory(&stage, &paths.tools.join("node"))?;
        files::remove_managed(&stage, cache)?;
        fs::remove_file(archive)?;
    }
    if find(paths, "git", "git.exe").is_none() {
        let a = asset(network, "git-for-windows/git", r"^MinGit-.*-64-bit\.zip$")?;
        let archive = cache.join(&a.name);
        network.asset(&a, &archive, job)?;
        let stage = cache.join(uuid::Uuid::new_v4().simple().to_string());
        files::extract_zip(&archive, &stage, job)?;
        merge_directory(&stage, &paths.tools.join("git"))?;
        files::remove_managed(&stage, cache)?;
        fs::remove_file(archive)?;
    }
    let env = environment(paths)?;
    for (folder, exe) in [
        ("cmake", "cmake.exe"),
        ("perl", "perl.exe"),
        ("nasm", "nasm.exe"),
        ("llvm", "clang.exe"),
        ("node", "node.exe"),
        ("git", "git.exe"),
    ] {
        let exe_path = find(paths, folder, exe).context(format!("Missing tool: {folder}"))?;
        job.run(
            Command::new(exe_path)
                .arg(if folder == "perl" { "-v" } else { "--version" })
                .envs(&env),
            false,
        )?;
    }
    Ok(())
}
fn merge_directory(source: &Path, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    for e in fs::read_dir(source)? {
        let e = e?;
        let target = dest.join(e.file_name());
        if target.exists() {
            bail!("Tool destination already contains {}; clear the incomplete tool folder before retrying",target.display());
        }
        fs::rename(e.path(), target)?;
    }
    Ok(())
}
fn ensure_seven(paths: &Paths, network: &Network, cache: &Path, job: &Job) -> Result<PathBuf> {
    if let Some(s) = seven(paths) {
        return Ok(s);
    }
    let bootstrap = asset(network, "ip7z/7zip", r"^7zr\.exe$")?;
    let exe = cache.join("7zr.exe");
    network.asset(&bootstrap, &exe, job)?;
    let extra = asset(network, "ip7z/7zip", r"-extra\.7z$")?;
    let archive = cache.join(&extra.name);
    network.asset(&extra, &archive, job)?;
    job.run(
        Command::new(&exe)
            .arg("x")
            .arg(&archive)
            .arg(format!("-o{}", paths.tools.join("7zip").display()))
            .arg("-y"),
        false,
    )?;
    fs::remove_file(exe)?;
    fs::remove_file(archive)?;
    seven(paths).context("7-Zip setup failed")
}
fn install_cpp(network: &Network, cache: &Path, job: &Job) -> Result<()> {
    job.stage(
        "Setting up tools",
        None,
        "Installing Microsoft C++ tools; Windows may request administrator approval",
    );
    let exe = cache.join("vs-buildtools.exe");
    network.download("https://aka.ms/vs/stable/vs_buildtools.exe", &exe, job)?;
    platform::verify_microsoft_signature(&exe)?;
    job.check()?;
    platform::run_elevated(&exe,"--passive --wait --norestart --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended",job)?;
    if visual_cpp()?.is_none() {
        bail!("Microsoft C++ setup did not finish. Restart Windows if requested, then retry.");
    }
    fs::remove_file(exe)?;
    Ok(())
}

#[cfg(test)]
mod build_path_tests {
    use super::*;
    #[test]
    fn compiler_alias_preserves_file_identity_and_reports_unsupported_long_paths() {
        let root = std::env::temp_dir().join(uuid::Uuid::new_v4().to_string());
        fs::create_dir_all(&root).unwrap();
        let file = root.join("compiler-path-test.txt");
        fs::write(&file, b"same file").unwrap();
        assert_eq!(
            fs::read(compiler_path(&file).unwrap()).unwrap(),
            b"same file"
        );
        let nonexistent = root.join("long-name".repeat(25)).join("missing");
        let error = compiler_path(&nonexistent).unwrap_err().to_string();
        assert!(error.contains("short folder alias is unavailable"));
        assert!(error.contains("Settings > Folders"));
        fs::remove_dir_all(root).unwrap();
    }
}
