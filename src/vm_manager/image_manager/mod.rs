use std::path::PathBuf;

use anyhow::Error;
use serde::{Deserialize, Serialize};

pub mod folder_manager;
pub mod url_manager;

#[async_trait::async_trait]
pub trait ImageManager: Sync + Send {
    async fn find_kernel(&self, manifest: &ImageManifest) -> Result<Image, Error>;
    async fn find_rootfs(&self, manifest: &ImageManifest) -> Result<Image, Error>;
    async fn find_disk(&self, manifest: &ImageManifest) -> Result<Image, Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Image {
    pub id: String,
    pub path: PathBuf,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ImageManifest {
    pub id: String,
    pub location: String,
}
