use super::*;

/// Caller owns the operation lock and has saved the verified removal inventory.
/// Unknown snapshots and transaction recovery folders are intentionally preserved.
pub fn discard_uninstalled(
    paths: &Paths,
    removed: &Installed,
    removed_folder: &Path,
) -> Result<()> {
    ensure!(
        removed.install_kind == "portable",
        "Only portable snapshots may be removed"
    );
    let original = Path::new(&removed.path);
    let parent = original.parent().context("Missing installation parent")?;
    ensure!(
        removed_folder.parent() == Some(parent),
        "Uninstall recovery folder is outside the installation parent"
    );
    let name = removed_folder
        .file_name()
        .and_then(|n| n.to_str())
        .context("Invalid uninstall folder")?;
    let suffix = name
        .strip_prefix(&format!(".uninstall-{}-", removed.name))
        .context("Not an uninstall recovery folder")?;
    uuid::Uuid::parse_str(suffix)?;
    portable::validate_path(removed_folder)?;
    files::no_links(removed_folder)?;
    let ownership: portable::Ownership = files::read_json(&removed_folder.join(portable::MARKER))?;
    ensure!(
        ownership.app == removed.name
            && crate::portable_mode::same_library(
                &ownership.library,
                &std::path::absolute(&paths.root)?
            ),
        "Uninstalled folder belongs to another library"
    );
    let Some(id) = ownership.installation_id else {
        return Ok(());
    };
    uuid::Uuid::parse_str(&id)?;
    let config = paths
        .read_inventory()?
        .context("Missing removal inventory")?;
    for copy in config
        .apps
        .iter()
        .chain(&config.installations)
        .filter(|c| !c.path.is_empty())
    {
        ensure!(
            copy.path != removed.path,
            "Installation is still in the inventory"
        );
        if copy.install_kind == "portable" {
            let marker = Path::new(&copy.path).join(portable::MARKER);
            if marker.exists() {
                let owner: portable::Ownership = files::read_json(&marker)?;
                ensure!(
                    owner.installation_id.as_ref() != Some(&id),
                    "Installation identity is still in use"
                );
            }
        }
    }
    let Some(point) = read_point(paths, &id)? else {
        return cleanup::prune_empty(paths);
    };
    ensure!(
        point.app == removed.name,
        "Previous version belongs to another application"
    );
    let directory = root(paths, &id)?;
    let contents = directory.join(&point.id);
    files::inside(&contents, &directory)?;
    ensure!(
        manifest(&contents)? == point.files,
        "Previous version has changed; preserved for recovery"
    );
    fs::remove_file(directory.join("point.json"))?;
    files::remove_managed(&contents, &directory)?;
    cleanup::prune_empty(paths)
}
