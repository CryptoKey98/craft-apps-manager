//! Non-distributed Windows transaction probe. Production trust/download policy is unchanged.
use anyhow::{ensure, Context, Result};
use craft_apps_manager::{
    files,
    jobs::Job,
    manager_updates::{
        catalog::{verify_catalog, TrustContext},
        check_state::{check, verified_cached, CheckContext, CheckTrigger},
        handlers::archive,
        model::*,
        transaction::{self, SignedSelection},
        transport::{MetadataTransport, ValidatedEndpoint},
        trust::TrustStore,
    },
    model::{BuilderPreferences, Config, Paths, Theme},
    self_update,
};
use serde::{Deserialize, Serialize};
use std::{fs, io::Read, path::Path, path::PathBuf, time::Duration};
const VERSION: &str = env!("CRAFT_UPDATER_FIXTURE_VERSION");
const MARKER: &str = "Craft updater isolated fixture";
fn helper_name() -> &'static str {
    if cfg!(windows) {
        "update-helper.exe"
    } else {
        "update-helper"
    }
}

fn owned(path: &Path) -> Result<()> {
    files::no_links(path)?;
    ensure!(
        fs::read_to_string(path.join(".craft-updater-fixture"))? == MARKER,
        "Not an owned fixture folder"
    );
    Ok(())
}
fn trust() -> TrustContext {
    TrustContext {
        repository: "owner/manager".into(),
        channel: "test".into(),
        keys: vec![*include_bytes!(
            "../../tests/fixtures/updater-v2/test-public-key.bin"
        )],
    }
}
fn profile() -> ClientProfile {
    ClientProfile {
        repository: "owner/manager".into(),
        channel: "test".into(),
        version: VERSION.into(),
        revision: 1,
        platform: "windows".into(),
        architecture: craft_apps_manager::model::MANAGER_ARCH.into(),
        installation: InstallationKind::Portable,
        capabilities: [ZIP.into(), LAYOUT.into()].into_iter().collect(),
    }
}
fn worker_url(value: &str) -> Result<reqwest::Url> {
    ValidatedEndpoint::new(value)?;
    let url = reqwest::Url::parse(value)?;
    ensure!(
        url.host_str()
            .is_some_and(|h| h.starts_with("craft-apps-manager-updates-test.")
                && h.ends_with(".workers.dev"))
            && url.path() == "/"
            && url.port().is_none(),
        "Only the isolated test Worker is permitted"
    );
    Ok(url)
}
fn startup(data: &Path) -> Result<()> {
    owned(data)?;
    let config: Config = files::read_json(&data.join("settings.json"))?;
    ensure!(
        Paths::new(data.into(), Some(data.join("tools")))
            .read_preferences()?
            .theme
            == Theme::Light,
        "Fixture preference interpretation changed"
    );
    ensure!(
        !config.apps.is_empty()
            && config.apps.iter().chain(&config.installations).all(
                |a| Path::new(&a.path).is_dir() && Path::new(&a.path).join("app.exe").is_file()
            ),
        "Fixture app records/files are missing"
    );
    let migration = data.join("fixture-migration.json");
    if VERSION == "1.1.0" {
        files::write_json(
            &migration,
            &serde_json::json!({"schema":2,"migrated_by":"1.1.0"}),
        )?;
    }
    if VERSION == "1.2.0" {
        let state: serde_json::Value = files::read_json(&migration)?;
        ensure!(state["schema"] == 2, "Required bridge migration is missing");
    }
    files::write_json(
        &data.join("worker-fixture-receipt.json"),
        &serde_json::json!({"version":VERSION,"pid":std::process::id()}),
    )
}

#[derive(Serialize, Deserialize)]
struct Authorization {
    envelope: Vec<u8>,
    offer: UpdateOffer,
    source_hash: String,
    profile: ClientProfile,
    legacy: self_update::Plan,
}

fn download(worker: &reqwest::Url, package: &PackageRecord, dest: &Path) -> Result<()> {
    // Only this non-distributed fixture maps a signed asset to the test Worker.
    // The real downloader still permits only repository-bound GitHub assets.
    ensure!(package.size <= 25 * 1024 * 1024, "Test artifact too large");
    let url = worker.join(&format!("artifacts/{}-{}", package.sha256, package.asset))?;
    let client = reqwest::blocking::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()?;
    let mut response = client.get(url).send()?.error_for_status()?;
    ensure!(
        response
            .content_length()
            .is_none_or(|size| size == package.size),
        "Test artifact size mismatch"
    );
    let mut file = fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(dest)?;
    let bytes = std::io::copy(&mut response.by_ref().take(package.size + 1), &mut file)?;
    file.sync_all()?;
    drop(file);
    ensure!(
        bytes == package.size,
        "Test artifact exceeds/misses signed size"
    );
    craft_apps_manager::network::verify_asset(dest, &transaction::asset(package))
}

fn prepare(data: &Path, worker: &reqwest::Url, stage_only: bool) -> Result<()> {
    let source = std::env::current_exe()?.canonicalize()?;
    let home = source.parent().context("Missing fixture folder")?;
    owned(home)?;
    let context = trust();
    let transport = MetadataTransport::new()?;
    let endpoints = [ValidatedEndpoint::new(worker.join("v2/check")?.as_str())?];
    let client = profile();
    let now = chrono::Utc::now().timestamp();
    let result = check(
        &client,
        &CheckContext {
            root: data,
            trust: &context,
            endpoints: &endpoints,
            transport: &transport,
            now,
            trigger: CheckTrigger::BridgeRestart,
        },
    );
    let offer = match result {
        CheckOutcome::DirectUpdate(o) | CheckOutcome::BridgeUpdate(o) => o,
        other => anyhow::bail!("No fixture update: {other:?}"),
    };
    let catalog = verified_cached(data, &context, now)?;
    transaction::validate_offer(&catalog, &client, &offer)?;
    let package = catalog
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == offer.immediate_package_id)
        .context("Missing fixture package")?;
    ensure!(
        package.handler == ZIP,
        "Only Windows ZIP fixture packages supported"
    );
    let stage = home
        .join("runtime/self-update")
        .join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir_all(&stage)?;
    files::no_links(&stage)?;
    let artifact = stage.join("download.package");
    download(worker, package, &artifact)?;
    let job = Job::new(stage.join("fixture.log"), &BuilderPreferences::default());
    let payload = archive::stage(package, &artifact, &stage.join("payload"), &job)?;
    let identity = transaction::ExpectedIdentity {
        version: catalog.catalog().release(package).version.clone(),
        revision: package.revision,
        platform: package.platform.clone(),
        architecture: package.architecture.clone(),
    };
    transaction::verify_executable(&payload.entrypoint, &identity, &stage)?;
    let plan = self_update::Plan {
        expected_identity: Some(identity),
        layout: 0,
        msi: false,
        linux_package: false,
        parent_pid: std::process::id(),
        target: source.clone(),
        staged: payload.entrypoint,
        hash: payload.hash,
        root: data.into(),
        tools: data.join("tools"),
    };
    let authorization = Authorization {
        envelope: catalog.envelope().to_vec(),
        offer,
        source_hash: files::hash(&source)?,
        profile: client,
        legacy: plan,
    };
    files::write_json(&stage.join("fixture-authorization.json"), &authorization)?;
    files::write_json(&stage.join("worker-plan.json"), &authorization.legacy)?;
    fs::copy(&source, stage.join(helper_name()))?;
    if stage_only {
        println!("{}", stage.join("worker-plan.json").display());
        Ok(())
    } else {
        self_update::launch(&stage.join("worker-plan.json"))
    }
}

#[cfg(target_os = "windows")]
fn wait_parent(pid: u32) -> Result<()> {
    use windows::Win32::{
        Foundation::{CloseHandle, WAIT_OBJECT_0},
        System::Threading::{OpenProcess, WaitForSingleObject, PROCESS_ACCESS_RIGHTS},
    };
    ensure!(
        pid > 0 && pid != std::process::id(),
        "Invalid fixture parent"
    );
    unsafe {
        if let Ok(handle) = OpenProcess(PROCESS_ACCESS_RIGHTS(0x00100000), false, pid) {
            let status = WaitForSingleObject(handle, 60_000);
            let _ = CloseHandle(handle);
            ensure!(status == WAIT_OBJECT_0, "Fixture parent did not exit");
        }
    }
    Ok(())
}
#[cfg(not(target_os = "windows"))]
fn wait_parent(_pid: u32) -> Result<()> {
    anyhow::bail!("Worker replacement fixture currently supports Windows only")
}

fn apply(path: &Path) -> Result<()> {
    let stage = path.parent().context("No fixture stage")?.canonicalize()?;
    let auth_path = stage.join("fixture-authorization.json");
    ensure!(
        fs::metadata(&auth_path)?.len() <= 12 * 1024 * 1024,
        "Fixture plan too large"
    );
    files::no_links(&auth_path)?;
    let mut auth: Authorization = files::read_json(&auth_path)?;
    let legacy: self_update::Plan = files::read_json(path)?;
    ensure!(
        !legacy.msi
            && !legacy.linux_package
            && legacy.layout == 0
            && legacy
                .target
                .file_name()
                .is_some_and(|n| n == "fixture-manager.exe"),
        "Only fixture executable replacement is permitted"
    );
    ensure!(
        serde_json::to_value(&legacy)? == serde_json::to_value(&auth.legacy)?,
        "Fixture plan changed"
    );
    let home = legacy.target.parent().context("No fixture target")?;
    owned(home)?;
    owned(&legacy.root)?;
    files::inside(&stage, &home.join("runtime/self-update"))?;
    ensure!(
        std::env::current_exe()?.canonicalize()? == stage.join(helper_name()).canonicalize()?
            && files::hash(&std::env::current_exe()?)? == auth.source_hash,
        "Fixture helper identity mismatch"
    );
    ensure!(
        serde_json::to_value(profile())? == serde_json::to_value(&auth.profile)?,
        "Fixture source profile changed"
    );
    fs::write(stage.join("helper.pid"), std::process::id().to_string())?;
    wait_parent(legacy.parent_pid)?;
    let _lock = transaction::target_lock(&legacy.target)?;
    transaction::validate_source(&legacy.target, &auth.source_hash)?;
    let context = trust();
    let prior = TrustStore::new(&legacy.root, &context.repository, &context.channel).load()?;
    let catalog = verify_catalog(
        &auth.envelope,
        &context,
        chrono::Utc::now().timestamp(),
        prior.as_ref(),
    )?;
    transaction::validate_offer(&catalog, &profile(), &auth.offer)?;
    let package = catalog
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == auth.offer.immediate_package_id)
        .context("Missing fixture package")?;
    let artifact = stage.join("download.package");
    craft_apps_manager::network::verify_asset(&artifact, &transaction::asset(package))?;
    files::remove_managed(&stage.join("payload"), &stage)?;
    let job = Job::new(stage.join("verify.log"), &BuilderPreferences::default());
    let payload = archive::stage(package, &artifact, &stage.join("payload"), &job)?;
    transaction::verify_executable(
        &payload.entrypoint,
        legacy
            .expected_identity
            .as_ref()
            .context("Missing identity")?,
        &stage,
    )?;
    auth.legacy.staged = payload.entrypoint;
    auth.legacy.hash = payload.hash;
    files::write_json(path, &auth.legacy)?;
    let _authorization = transaction::authorize_commit(
        &legacy.root,
        &SignedSelection {
            catalog,
            offer: auth.offer,
        },
        &profile(),
        &context,
        chrono::Utc::now().timestamp(),
    )?;
    self_update::apply(path)
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|v| v == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    if args.iter().any(|v| v == "--build-identity") {
        println!(
            "{}",
            serde_json::json!({"version":VERSION,"revision":1,"target":env!("CRAFT_BUILD_TARGET")})
        );
        return Ok(());
    }
    if let Some(path) = arg("--apply-self-update") {
        return apply(&path);
    }
    ensure!(
        cfg!(target_os = "windows"),
        "Worker transaction fixture currently supports Windows only"
    );
    let data = arg("--root")
        .context("Fixture needs --root")?
        .canonicalize()?;
    let home = std::env::current_exe()?.canonicalize()?;
    owned(home.parent().context("Missing fixture home")?)?;
    startup(&data)?;
    if let Some(worker) = arg("--worker") {
        ensure!(
            args.iter().any(|v| v == "--approve"),
            "Each fixture hop needs explicit approval"
        );
        prepare(
            &data,
            &worker_url(worker.to_str().context("Invalid Worker URL")?)?,
            args.iter().any(|v| v == "--stage-only"),
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn startup_reads_production_settings_roles() {
        let data = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("update-service/.wrangler")
            .join(format!("settings-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&data).unwrap();
        fs::write(data.join(".craft-updater-fixture"), MARKER).unwrap();
        let app = data.join("app.exe");
        fs::write(&app, b"preserved app").unwrap();
        files::write_json(&data.join("settings.json"), &serde_json::json!({"appsRoot":data,"apps":[{"name":"fixture-app","version":"1.0.0","path":data}],"installations":[]})).unwrap();
        files::write_json(
            &data.join("manager-settings.json"),
            &serde_json::json!({"theme":"light"}),
        )
        .unwrap();
        let result = startup(&data);
        fs::remove_dir_all(&data).unwrap();
        result.expect("Fixture must interpret the real manager's settings filenames");
    }
    #[test]
    fn refuses_production_or_untrusted_artifact_origins() {
        for url in [
            "https://updates.example/",
            "http://craft-apps-manager-updates-test.example.workers.dev/",
            "https://craft-apps-manager-updates-test.example.workers.dev/other",
            "https://user:secret@craft-apps-manager-updates-test.example.workers.dev/",
        ] {
            assert!(worker_url(url).is_err());
        }
        assert!(worker_url("https://craft-apps-manager-updates-test.example.workers.dev/").is_ok());
    }
}
