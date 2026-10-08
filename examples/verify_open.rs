fn main() -> anyhow::Result<()> {
    for path in std::env::args_os().skip(1) {
        craft_apps_manager::platform::open(std::path::Path::new(&path))?;
    }
    Ok(())
}
