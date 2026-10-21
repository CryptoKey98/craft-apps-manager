use anyhow::{ensure, Result};
pub use craft_updater_service::transport::*;
use std::{io::Read, time::Duration};
pub fn download_asset(
    package: &super::model::PackageRecord,
    dest: &std::path::Path,
    job: &crate::jobs::Job,
) -> Result<()> {
    use std::io::Write;
    let initial = reqwest::Url::parse(&package.url)?;
    ensure!(asset_host_allowed(&initial), "Untrusted package origin");
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(
            env!("CRAFT_UPDATE_DOWNLOAD_TIMEOUT").parse()?,
        ))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 10 || !asset_host_allowed(attempt.url()) {
                attempt.error("Untrusted package redirect")
            } else {
                attempt.follow()
            }
        }))
        .user_agent("CraftAppsManager-Updater/2")
        .build()?;
    job.check()?;
    let result = (|| {
        let mut response = client.get(initial).send()?.error_for_status()?;
        ensure!(
            response.content_length().is_none_or(|n| n == package.size),
            "Package size mismatch"
        );
        let mut output = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?;
        let mut total = 0u64;
        let mut buffer = [0; 65536];
        loop {
            job.check()?;
            let n = response.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            total += n as u64;
            ensure!(total <= package.size, "Package exceeds signed size");
            output.write_all(&buffer[..n])?;
            job.stage(
                "Downloading manager update",
                Some(total as f32 / package.size as f32),
                format!("{} MB", total / 1048576),
            );
        }
        output.sync_all()?;
        drop(output);
        job.check()?;
        crate::network::verify_asset(dest, &super::transaction::asset(package))
    })();
    if result.is_err() && dest.is_file() {
        let _ = std::fs::remove_file(dest);
    }
    result
}
