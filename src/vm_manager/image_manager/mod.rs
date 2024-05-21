use std::path::PathBuf;

use anyhow::Error;
use serde::{Deserialize, Serialize};

pub mod folder_manager;

#[async_trait::async_trait]
pub trait ImageManager: Sync + Send {
    async fn find_kernel(&self, id: &str) -> Result<Image, Error>;
    async fn find_rootfs(&self, id: &str) -> Result<Image, Error>;
    async fn find_disk(&self, id: &str) -> Result<Image, Error>;
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Image {
    pub id: String,
    pub name: String,
    pub path: PathBuf,
}
