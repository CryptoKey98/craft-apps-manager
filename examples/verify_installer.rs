fn main() -> anyhow::Result<()> {
    let app = craft_apps_manager::installers::detect("filmcraft")?
        .ok_or_else(|| anyhow::anyhow!("FilmCraft not detected"))?;
    println!(
        "{} {} at {} ({})",
        app.name, app.version, app.path, app.product_code
    );
    Ok(())
}
