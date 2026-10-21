//! Publisher-side validation uses exactly the client's routing implementation.
use anyhow::{ensure, Context, Result};
use craft_apps_manager::manager_updates::{
    model::{Catalog, ClientProfile},
    routing::validate_publishing_routes,
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    ensure!(
        args.len() == 3,
        "Usage: update-catalog-tool CATALOG PROFILES"
    );
    let catalog: Catalog = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let profiles: Vec<ClientProfile> = serde_json::from_slice(&std::fs::read(&args[2])?)?;
    ensure!(!profiles.is_empty(), "Supported profiles must not be empty");
    validate_publishing_routes(&catalog, &profiles)
        .context("Supported client would be stranded")?;
    println!(
        "Catalog and {} supported client profiles validated",
        profiles.len()
    );
    Ok(())
}
