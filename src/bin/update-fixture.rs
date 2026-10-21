//! Non-distributed test product. It never reads production credentials or feeds.
use anyhow::{ensure, Context, Result};
use craft_apps_manager::{
    files,
    jobs::Job,
    manager_updates::{
        catalog::{verify_catalog, TrustContext},
        handlers::archive,
        model::*,
        routing::select_update,
    },
    model::BuilderPreferences,
    self_update,
};
use std::{
    fs,
    path::{Path, PathBuf},
};
const VERSION: &str = env!("CRAFT_UPDATER_FIXTURE_VERSION");
const MARKER: &str = "Craft updater isolated fixture";
fn owned(root: &Path) -> Result<()> {
    ensure!(
        fs::read_to_string(root.join(".craft-updater-fixture"))? == MARKER,
        "Not an isolated fixture root"
    );
    files::no_links(root)?;
    Ok(())
}
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    let arg = |name: &str| {
        args.iter()
            .position(|v| v == name)
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from)
    };
    if let Some(plan) = arg("--apply-self-update") {
        fs::write(
            plan.parent().context("No helper stage")?.join("helper.pid"),
            std::process::id().to_string(),
        )?;
        return self_update::apply(&plan);
    }
    let data = arg("--root").context("Fixture needs an owned --root")?;
    owned(&data)?;
    let executable = std::env::current_exe()?.canonicalize()?;
    let home = executable.parent().context("No fixture folder")?;
    owned(home)?;
    files::write_bytes(
        &data.join("fixture-receipt.json"),
        &serde_json::to_vec(
            &serde_json::json!({"version":VERSION,"pid":std::process::id(),"tar":cfg!(feature="updater-test-tar")}),
        )?,
    )?;
    let Some(scenario) = arg("--scenario") else {
        return Ok(());
    };
    let base = scenario.parent().context("No fixture scenario folder")?;
    owned(base)?;
    #[derive(serde::Deserialize)]
    struct Scenario {
        catalog: String,
        artifact_dir: String,
        approve: bool,
    }
    let scenario: Scenario = serde_json::from_slice(&fs::read(&scenario)?)?;
    ensure!(
        scenario.approve,
        "Fixture update needs explicit test approval"
    );
    relative(&scenario.catalog)?;
    relative(&scenario.artifact_dir)?;
    let bytes = fs::read(base.join(scenario.catalog))?;
    // Public test key for seed [7;32]; never part of the production manager's trust context.
    let key = include_bytes!("../../tests/fixtures/updater-v2/test-public-key.bin");
    let trust = TrustContext {
        repository: "owner/manager".into(),
        channel: "stable".into(),
        keys: vec![*key],
    };
    let verified = verify_catalog(&bytes, &trust, chrono::Utc::now().timestamp(), None)?;
    let mut capabilities = std::collections::BTreeSet::from([ZIP.into(), LAYOUT.into()]);
    #[cfg(feature = "updater-test-tar")]
    {
        capabilities.insert(TAR.into());
    }
    #[cfg(not(feature = "updater-test-tar"))]
    let _ = &mut capabilities;
    let profile = ClientProfile {
        repository: trust.repository.clone(),
        channel: trust.channel.clone(),
        version: VERSION.into(),
        revision: 1,
        platform: "linux".into(),
        architecture: "x64".into(),
        installation: InstallationKind::Portable,
        capabilities,
    };
    let offer = match select_update(&verified, &profile)? {
        CheckOutcome::DirectUpdate(o) | CheckOutcome::BridgeUpdate(o) => o,
        _ => anyhow::bail!("No fixture update"),
    };
    let p = verified
        .catalog()
        .packages
        .iter()
        .find(|p| p.id == offer.immediate_package_id)
        .context("Missing fixture package")?;
    ensure!(
        p.handler == ZIP || cfg!(feature = "updater-test-tar") && p.handler == TAR,
        "Handler absent from fixture build"
    );
    let archive = base.join(scenario.artifact_dir).join(&p.asset);
    files::inside(&archive, base)?;
    craft_apps_manager::network::verify_asset(
        &archive,
        &craft_apps_manager::manager_updates::transaction::asset(p),
    )?;
    let stage = home
        .join("runtime/self-update")
        .join(uuid::Uuid::new_v4().simple().to_string());
    fs::create_dir_all(&stage)?;
    let job = Job::new(stage.join("fixture.log"), &BuilderPreferences::default());
    let payload = archive::stage(p, &archive, &stage.join("payload"), &job)?;
    let plan = self_update::Plan {
        expected_identity: None,
        layout: 0,
        msi: false,
        linux_package: false,
        parent_pid: std::process::id(),
        target: executable.clone(),
        staged: payload.entrypoint,
        hash: payload.hash,
        root: data.clone(),
        tools: data.join("tools"),
    };
    let path = stage.join("plan.json");
    files::write_json(&path, &plan)?;
    fs::copy(
        executable,
        stage.join(if cfg!(windows) {
            "update-helper.exe"
        } else {
            "update-helper"
        }),
    )?;
    self_update::launch(&path)
}
