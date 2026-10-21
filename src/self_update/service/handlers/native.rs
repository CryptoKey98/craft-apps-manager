use anyhow::{ensure, Result};
pub fn validate_identity(
    name: &str,
    version: &str,
    architecture: &str,
    expected_version: &str,
    expected_architecture: &str,
) -> Result<()> {
    ensure!(
        name == format!("Craft Apps Manager ({expected_architecture})")
            && version == expected_version
            && architecture == expected_architecture,
        "Native package identity/version/architecture mismatch"
    );
    Ok(())
}
#[cfg(target_os = "windows")]
pub fn validate_msi(file: &std::path::Path, version: &str, architecture: &str) -> Result<()> {
    let (name, actual, arch, family) = crate::installers::manager_msi_metadata(file)?;
    validate_identity(&name, &actual, &arch, version, architecture)?;
    let expected = if architecture == "x86" {
        "{706B76C5-B377-48B9-AC61-E65986704A2F}"
    } else {
        "{964F2264-6758-4DE9-BBE9-1C74B295E47B}"
    };
    ensure!(
        family.eq_ignore_ascii_case(expected),
        "MSI upgrade family mismatch"
    );
    Ok(())
}
