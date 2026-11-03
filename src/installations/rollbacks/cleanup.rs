use super::*;

/// Caller holds the installation operation lock. Only remove empty, owned UUID
/// directories; non-recursive deletion preserves recovery and concurrent content.
pub(super) fn prune_empty(paths: &Paths) -> Result<()> {
    for base in [
        paths.at("rollbacks"),
        paths.at("runtime/rollback-transactions"),
    ] {
        if !base.exists() {
            continue;
        }
        files::inside(&base, &paths.root)?;
        portable::validate_path(&base)?;
        for entry in fs::read_dir(&base)? {
            let entry = entry?;
            if uuid::Uuid::parse_str(&entry.file_name().to_string_lossy()).is_err() {
                continue;
            }
            let directory = entry.path();
            files::inside(&directory, &base)?;
            portable::validate_path(&directory)?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            match fs::remove_dir(&directory) {
                Ok(()) => {}
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        match fs::remove_dir(&base) {
            Ok(()) => {}
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
                ) => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}
