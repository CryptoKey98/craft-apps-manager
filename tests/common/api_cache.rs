//! Seed an offline response in the active credential scope; no HTTP or credential writes.
use anyhow::Result;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::path::Path;

pub fn write(root: &Path, url: &str, envelope: &Value) -> Result<()> {
    let token = if url.starts_with("https://api.github.com/") {
        ["CRAFT_GITHUB_TOKEN", "GITHUB_TOKEN"]
            .into_iter()
            .find_map(|name| {
                std::env::var(name)
                    .ok()
                    .filter(|value| !value.trim().is_empty())
                    .map(|value| value.trim().to_owned())
            })
            .or_else(|| {
                keyring::Entry::new("Craft Apps Manager", "github-api-token")
                    .ok()?
                    .get_password()
                    .ok()
            })
    } else {
        None
    };
    let mut digest = Sha256::new();
    digest.update(url.as_bytes());
    if let Some(token) = token {
        digest.update(b"\0credential\0");
        digest.update(Sha256::digest(token.as_bytes()));
    }
    let key = format!("{:x}", digest.finalize());
    let bytes = serde_json::to_vec(&envelope["value"])?;
    let directory = root.join("runtime/api-cache");
    craft_apps_manager::files::write_bytes(&directory.join(format!("{key}.body")), &bytes)?;
    craft_apps_manager::files::write_json(
        &directory.join(format!("{key}.meta.json")),
        &json!({
            "version":1,"codec":"identity","body_hash":format!("{:x}",Sha256::digest(&bytes)),"decoded_len":bytes.len(),
            "at":envelope["at"],"accessed":envelope["at"],"etag":envelope["etag"],"generation":uuid::Uuid::new_v4().to_string()
        }),
    )
}
