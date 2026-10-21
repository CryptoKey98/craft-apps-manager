pub mod archive;
pub mod dmg;
pub mod native;
use std::path::PathBuf;
#[derive(Debug)]
pub struct StagedPayload {
    pub root: PathBuf,
    pub entrypoint: PathBuf,
    pub hash: String,
}
