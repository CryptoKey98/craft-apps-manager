//! Root-owned bundle transactions. Publication exchanges complete directories.
use super::*;
use std::{
    ffi::CString,
    os::{
        fd::AsRawFd,
        unix::{
            ffi::OsStrExt,
            fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
        },
    },
};

#[derive(Clone, Copy, PartialEq, Eq)]
struct Node {
    dev: u64,
    ino: u64,
}
fn node(path: &Path) -> Result<Node> {
    let m = fs::symlink_metadata(path)?;
    ensure!(
        m.is_dir() && !m.file_type().is_symlink(),
        "Bundle directory was replaced"
    );
    Ok(Node {
        dev: m.dev(),
        ino: m.ino(),
    })
}
fn directory(path: &Path) -> Result<fs::File> {
    Ok(fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(path)?)
}

struct Exchange {
    parent: fs::File,
    stage: fs::File,
    target: PathBuf,
    home: PathBuf,
    original: Node,
    incoming: Option<Node>,
    published: bool,
    conflict: bool,
}
impl Exchange {
    fn new(target: &Path, home: &Path) -> Result<Self> {
        let parent = directory(target.parent().context("Missing app parent")?)?;
        let stage = directory(home)?;
        ensure!(
            parent.metadata()?.dev() == stage.metadata()?.dev(),
            "Cross-device bundle stage"
        );
        Ok(Self {
            parent,
            stage,
            target: target.into(),
            home: home.into(),
            original: node(target)?,
            incoming: None,
            published: false,
            conflict: false,
        })
    }
    fn anchored(&self) -> Result<()> {
        let parent = self.target.parent().context("Missing target parent")?;
        let held = self.parent.metadata()?;
        ensure!(
            node(parent)?
                == (Node {
                    dev: held.dev(),
                    ino: held.ino()
                }),
            "Application parent changed"
        );
        let held = self.stage.metadata()?;
        ensure!(
            node(&self.home)?
                == (Node {
                    dev: held.dev(),
                    ino: held.ino()
                }),
            "Private exchange stage changed"
        );
        Ok(())
    }
    fn swap(&self) -> Result<()> {
        let target = CString::new(
            self.target
                .file_name()
                .context("No bundle name")?
                .as_bytes(),
        )?;
        let result = unsafe {
            libc::renameatx_np(
                self.parent.as_raw_fd(),
                target.as_ptr(),
                self.stage.as_raw_fd(),
                c"incoming.app".as_ptr(),
                libc::RENAME_SWAP,
            )
        };
        ensure!(
            result == 0,
            "Atomic bundle exchange failed: {}",
            std::io::Error::last_os_error()
        );
        Ok(())
    }
    fn publish(&mut self) -> Result<()> {
        self.publish_with(Self::swap)
    }
    fn publish_with(&mut self, exchange: impl FnOnce(&Self) -> Result<()>) -> Result<()> {
        ensure!(
            !self.published && self.incoming.is_none(),
            "Bundle already published"
        );
        self.anchored()?;
        ensure!(
            node(&self.target)? == self.original,
            "Original app directory changed"
        );
        self.incoming = Some(node(&self.home.join("incoming.app"))?);
        exchange(self)?;
        self.published = true; // Record ownership before any fallible durability check.
        if node(&self.home.join("incoming.app")).ok() != Some(self.original) {
            self.conflict = true;
            // A different installer won the child-name race. Restore its displaced
            // directory only while the live target is still the bundle we published.
            if node(&self.target).ok() == self.incoming {
                self.swap()?;
                self.published = false;
            }
            anyhow::bail!(
                "Concurrent app replacement; recovery retained at {}",
                self.home.display()
            );
        }
        self.parent.sync_all()?;
        self.stage.sync_all()?;
        Ok(())
    }
    fn verify(&self) -> Result<()> {
        ensure!(
            !self.conflict,
            "Conflicting bundle recovery must be retained"
        );
        self.anchored()?;
        ensure!(
            self.published && Some(node(&self.target)?) == self.incoming,
            "Published application was changed by another process"
        );
        ensure!(
            node(&self.home.join("incoming.app"))? == self.original,
            "Displaced bundle changed; it is not owned cleanup authority"
        );
        Ok(())
    }
    fn rollback(&mut self) -> Result<()> {
        if !self.published {
            return Ok(());
        }
        self.verify()?;
        ensure!(
            node(&self.home.join("incoming.app"))? == self.original,
            "Recovery app changed"
        );
        self.swap()?;
        if node(&self.home.join("incoming.app")).ok() != self.incoming {
            self.conflict = true;
            if node(&self.target).ok() == Some(self.original) {
                self.swap()?;
            }
            anyhow::bail!(
                "Concurrent rollback replacement; recovery retained at {}",
                self.home.display()
            );
        }
        self.published = false;
        self.incoming = None;
        self.parent.sync_all()?;
        Ok(())
    }
}

pub(super) struct RootBundleTransaction {
    exchange: Exchange,
    current_hash: String,
    pending: Option<(usize, String)>,
    staged: Option<(usize, String)>,
    next: usize,
}
impl RootBundleTransaction {
    pub(super) fn open(target_executable: &Path, caller_uid: u32) -> Result<Self> {
        ensure!(
            unsafe { libc::geteuid() } == 0 && caller_uid > 0,
            "Invalid privileged caller"
        );
        let target = crate::self_update::app_bundle(target_executable)?;
        let parent = target.parent().context("No app parent")?;
        super::macos::protected_parent(parent)?;
        crate::self_update::verify_bundle(&target)?;
        files::no_links(&target)?;
        let home = parent.join(format!(
            ".craft-manager-root-{}",
            uuid::Uuid::new_v4().simple()
        ));
        fs::DirBuilder::new().mode(0o700).create(&home)?;
        let result = (|| {
            Ok(Self {
                exchange: Exchange::new(&target, &home)?,

                current_hash: files::hash(target_executable)?,
                pending: None,
                staged: None,
                next: 0,
            })
        })();
        if result.is_err() {
            let _ = fs::remove_dir(&home);
        }
        result
    }
    pub(super) fn stage(
        &mut self,
        index: usize,
        archive: &Path,
        package: &PackageRecord,
        catalog: &VerifiedCatalog,
    ) -> Result<()> {
        ensure!(
            self.pending.is_none() && self.staged.is_none() && index == self.next,
            "Unapproved bundle stage order"
        );
        self.exchange.anchored()?;
        super::macos::validate(archive, package, catalog)?;
        let stage = self.exchange.home.join(format!("payload-{index}"));
        let job = Job::new(
            self.exchange.home.join("extract.log"),
            &crate::model::BuilderPreferences::default(),
        );
        let payload = match package.handler.as_str() {
            super::super::model::ZIP | super::super::model::TAR => {
                super::super::handlers::archive::stage(package, archive, &stage, &job)?
            }
            super::super::model::DMG => {
                super::super::handlers::dmg::stage(package, archive, &stage, &job)?
            }
            _ => anyhow::bail!("Unsupported privileged bundle format"),
        };
        ensure!(
            crate::self_update::staged_executable(&payload.root) == payload.entrypoint,
            "Bundle entrypoint differs from signed layout"
        );
        verify_identity(&payload.root, package, catalog)?;
        // Normalize permissions so the ordinary user can verify and launch the new bundle.
        use std::os::unix::fs::PermissionsExt;
        for entry in walkdir::WalkDir::new(&payload.root).follow_links(false) {
            let entry = entry?;
            ensure!(!entry.file_type().is_symlink(), "Linked bundle");
            let mode = if entry.file_type().is_dir() {
                0o755
            } else {
                entry.metadata()?.mode() & 0o111 | 0o644
            };
            fs::set_permissions(entry.path(), fs::Permissions::from_mode(mode))?;
        }
        fs::rename(payload.root, self.exchange.home.join("incoming.app"))?;
        self.staged = Some((index, payload.hash));
        Ok(())
    }
    pub(super) fn publish(&mut self, index: usize, expected_source_hash: &str) -> Result<String> {
        super::macos::protected_parent(self.exchange.target.parent().context("No app parent")?)?;
        ensure!(
            self.pending.is_none()
                && self.staged.as_ref().is_some_and(|(i, _)| *i == index)
                && self.current_hash == expected_source_hash
                && files::hash(&crate::self_update::staged_executable(
                    &self.exchange.target
                ))? == expected_source_hash,
            "Bundle source or order changed"
        );
        crate::self_update::verify_bundle(&self.exchange.target)?;
        let (_, hash) = self.staged.take().context("Missing staged bundle")?;
        // Retain state even if the exchange succeeds but the durability check fails.
        self.pending = Some((index, hash.clone()));
        self.exchange.publish()?;
        self.verify(index)?;
        Ok(hash)
    }
    pub(super) fn verify(&self, index: usize) -> Result<()> {
        let (pending, hash) = self.pending.as_ref().context("No pending bundle")?;
        ensure!(*pending == index, "Wrong pending bundle");
        self.exchange.verify()?;
        ensure!(
            files::hash(&crate::self_update::staged_executable(
                &self.exchange.target
            ))? == *hash,
            "Published executable changed"
        );
        crate::self_update::verify_bundle(&self.exchange.target)
    }
    pub(super) fn acknowledge(&mut self, index: usize) -> Result<Vec<String>> {
        self.verify(index)?;
        let previous = self.exchange.home.join("incoming.app");
        ensure!(
            files::hash(&crate::self_update::staged_executable(&previous))? == self.current_hash,
            "Previous bundle changed; refusing cleanup"
        );
        crate::self_update::verify_bundle(&previous)?;
        let old = self.exchange.home.join(format!("previous-{index}.app"));
        fs::rename(self.exchange.home.join("incoming.app"), &old)?;
        let (_, hash) = self.pending.take().context("No pending bundle")?;
        self.current_hash = hash;
        self.next += 1;
        self.exchange.original = node(&self.exchange.target)?;
        self.exchange.published = false;
        self.exchange.incoming = None;
        match fs::remove_dir_all(&old) {
            Ok(()) => Ok(Vec::new()),
            Err(e) => Ok(vec![format!(
                "Update verified; previous bundle retained at {}: {e}",
                old.display()
            )]),
        }
    }
    pub(super) fn rollback_pending(&mut self) -> Result<()> {
        ensure!(
            !self.exchange.conflict,
            "Conflicting recovery retained at {}",
            self.exchange.home.display()
        );
        if self.exchange.published {
            let (index, _) = self.pending.as_ref().context("Lost recovery state")?;
            self.verify(*index)?;
            let previous = self.exchange.home.join("incoming.app");
            ensure!(
                files::hash(&crate::self_update::staged_executable(&previous))?
                    == self.current_hash,
                "Previous bundle changed; recovery retained"
            );
            crate::self_update::verify_bundle(&previous)?;
            self.exchange.rollback()?;
        }
        self.pending = None;
        Ok(())
    }
}
impl Drop for RootBundleTransaction {
    fn drop(&mut self) {
        if self.rollback_pending().is_ok() && !self.exchange.published {
            let _ = fs::remove_dir_all(&self.exchange.home);
        }
        // Unsafe rollback deliberately retains the sole old bundle in root-private recovery.
    }
}
fn verify_identity(
    bundle: &Path,
    package: &PackageRecord,
    catalog: &VerifiedCatalog,
) -> Result<()> {
    crate::self_update::verify_bundle(bundle)?;
    ensure!(
        crate::platform::bundle_value(bundle, "CFBundleShortVersionString").as_deref()
            == Some(catalog.catalog().release(package).version.as_str())
            && crate::platform::bundle_value(bundle, "CFBundleVersion")
                .and_then(|s| s.parse::<u32>().ok())
                == Some(package.revision),
        "Bundle version/revision mismatch"
    );
    let output = super::macos::system_command("/usr/bin/lipo")
        .arg("-archs")
        .arg(crate::self_update::staged_executable(bundle))
        .output()?;
    let arch = match package.architecture.as_str() {
        "x64" => "x86_64",
        "arm64" => "arm64",
        _ => anyhow::bail!("Unsupported Mach-O architecture"),
    };
    ensure!(
        output.status.success()
            && std::str::from_utf8(&output.stdout)?
                .split_whitespace()
                .any(|v| v == arch),
        "Bundle architecture mismatch"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_in_the_final_swap_interval_is_preserved() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-swap-window-{}", uuid::Uuid::new_v4()));
        let target = root.join("Manager.app");
        let stage = root.join("stage");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(stage.join("incoming.app")).unwrap();
        fs::write(target.join("identity"), b"old").unwrap();
        fs::write(stage.join("incoming.app/identity"), b"our-new").unwrap();
        let mut exchange = Exchange::new(&target, &stage).unwrap();
        let outcome = exchange.publish_with(|exchange| {
            fs::rename(&target, root.join("another-installer-backup.app"))?;
            fs::create_dir(&target)?;
            fs::write(target.join("identity"), b"third-party")?;
            exchange.swap()
        });
        assert!(outcome.is_err());
        assert_eq!(fs::read(target.join("identity")).unwrap(), b"third-party");
        assert_eq!(
            fs::read(root.join("another-installer-backup.app/identity")).unwrap(),
            b"old"
        );
        assert_eq!(
            fs::read(stage.join("incoming.app/identity")).unwrap(),
            b"our-new"
        );
        assert!(exchange.verify().is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn changed_displaced_bundle_cannot_be_accepted_for_acknowledgement() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-displaced-{}", uuid::Uuid::new_v4()));
        let target = root.join("Manager.app");
        let stage = root.join("stage");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(stage.join("incoming.app")).unwrap();
        let mut exchange = Exchange::new(&target, &stage).unwrap();
        exchange.publish().unwrap();
        fs::rename(stage.join("incoming.app"), root.join("original.app")).unwrap();
        fs::create_dir(stage.join("incoming.app")).unwrap();
        assert!(
            exchange.verify().is_err(),
            "A different displaced bundle must not become deletion authority"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn exchange_is_atomic_and_rollback_preserves_a_substituted_target() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-exchange-{}", uuid::Uuid::new_v4()));
        let target = root.join("Manager.app");
        let stage = root.join("stage");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(stage.join("incoming.app")).unwrap();
        fs::write(target.join("identity"), "old").unwrap();
        fs::write(stage.join("incoming.app/identity"), "new").unwrap();
        let mut exchange = Exchange::new(&target, &stage).unwrap();
        exchange.publish().unwrap();
        assert_eq!(fs::read_to_string(target.join("identity")).unwrap(), "new");
        assert!(exchange.publish().is_err());
        exchange.rollback().unwrap();
        assert_eq!(fs::read_to_string(target.join("identity")).unwrap(), "old");
        exchange.publish().unwrap();
        fs::rename(&target, root.join("changed.app")).unwrap();
        fs::create_dir(&target).unwrap();
        fs::write(target.join("identity"), "third-party").unwrap();
        assert!(exchange.rollback().is_err());
        assert_eq!(
            fs::read_to_string(stage.join("incoming.app/identity")).unwrap(),
            "old"
        );
        assert_eq!(
            fs::read_to_string(target.join("identity")).unwrap(),
            "third-party"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn parent_substitution_and_linked_incoming_are_rejected_without_mutation() {
        use std::os::unix::fs::symlink;
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-exchange-race-{}", uuid::Uuid::new_v4()));
        let parent = root.join("parent");
        let target = parent.join("Manager.app");
        let stage = parent.join("stage");
        let outside = root.join("unrelated");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(&stage).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(target.join("identity"), b"old").unwrap();
        fs::write(outside.join("identity"), b"unrelated").unwrap();
        let mut exchange = Exchange::new(&target, &stage).unwrap();
        symlink(&outside, stage.join("incoming.app")).unwrap();
        assert!(exchange.publish().is_err());
        fs::remove_file(stage.join("incoming.app")).unwrap();
        fs::create_dir(stage.join("incoming.app")).unwrap();
        fs::rename(&parent, root.join("moved-parent")).unwrap();
        fs::create_dir(&parent).unwrap();
        assert!(exchange.publish().is_err());
        assert_eq!(
            fs::read(root.join("moved-parent/Manager.app/identity")).unwrap(),
            b"old"
        );
        assert_eq!(fs::read(outside.join("identity")).unwrap(), b"unrelated");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn unsafe_drop_retains_the_only_recovery_bundle() {
        let root = std::env::temp_dir()
            .canonicalize()
            .unwrap()
            .join(format!("craft-recovery-{}", uuid::Uuid::new_v4()));
        let target = root.join("Manager.app");
        let stage = root.join("stage");
        fs::create_dir_all(&target).unwrap();
        fs::create_dir_all(stage.join("incoming.app")).unwrap();
        fs::write(target.join("old"), b"old").unwrap();
        let mut exchange = Exchange::new(&target, &stage).unwrap();
        exchange.publish().unwrap();
        fs::rename(&target, root.join("new.app")).unwrap();
        fs::create_dir(&target).unwrap();
        let transaction = RootBundleTransaction {
            exchange,
            current_hash: "old".into(),
            pending: Some((0, "new".into())),
            staged: None,
            next: 0,
        };
        drop(transaction);
        assert_eq!(fs::read(stage.join("incoming.app/old")).unwrap(), b"old");
        assert!(target.is_dir());
        fs::remove_dir_all(root).unwrap();
    }
}
