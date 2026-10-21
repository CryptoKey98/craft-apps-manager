#![cfg(feature = "service-updater")]
mod common_updater;
use common_updater::*;
use craft_apps_manager::{
    jobs::Job,
    manager_updates::handlers::dmg::{stage_with_tools, DmgTools, Mount},
    model::BuilderPreferences,
};
struct Tools {
    root: std::path::PathBuf,
    detached: bool,
    fail: bool,
}
impl DmgTools for Tools {
    fn attach(&mut self, _: &std::path::Path) -> anyhow::Result<Mount> {
        Ok(Mount {
            device: "/dev/disk99".into(),
            path: self.root.clone(),
        })
    }
    fn detach(&mut self, _: &Mount) -> anyhow::Result<()> {
        self.detached = true;
        anyhow::ensure!(!self.fail, "detach failed");
        Ok(())
    }
}
#[test]
fn copies_declared_bundle_only() {
    let temp = Temp::new();
    let mount = temp.0.join("mount");
    std::fs::create_dir_all(mount.join("Manager.app/Contents/MacOS")).unwrap();
    std::fs::write(mount.join("Manager.app/Contents/MacOS/manager"), b"payload").unwrap();
    std::fs::write(mount.join("unrelated"), b"ignore").unwrap();
    let mut p = catalog().packages[0].clone();
    p.layout.root = "Manager.app".into();
    p.layout.entrypoint = "Contents/MacOS/manager".into();
    let mut tools = Tools {
        root: mount,
        detached: false,
        fail: false,
    };
    let result = stage_with_tools(
        &p,
        &temp.0.join("image"),
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default()),
        &mut tools,
    )
    .unwrap();
    assert!(result.entrypoint.is_file());
    assert!(tools.detached);
    assert!(!temp.0.join("stage/unrelated").exists());
}
#[test]
fn detach_failure_retains_cleanup_record() {
    let temp = Temp::new();
    let mut tools = Tools {
        root: temp.0.clone(),
        detached: false,
        fail: true,
    };
    assert!(stage_with_tools(
        &catalog().packages[0],
        &temp.0.join("image"),
        &temp.0.join("stage"),
        &Job::new(temp.0.join("a.log"), &BuilderPreferences::default()),
        &mut tools
    )
    .is_err());
    assert!(tools.detached);
    assert!(temp.0.join("stage/dmg-cleanup.json").exists());
}
