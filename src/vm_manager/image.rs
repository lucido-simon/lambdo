use std::path::PathBuf;

use anyhow::Error;
use serde::{Deserialize, Serialize};
use tracing::trace;

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

pub struct FolderImageManager {
    pub path: PathBuf,
}

impl FolderImageManager {
    pub fn new(path: String) -> Self {
        Self { path: path.into() }
    }
}

#[async_trait::async_trait]
impl ImageManager for FolderImageManager {
    async fn find_disk(&self, id: &str) -> Result<Image, Error> {
        trace!("find_disk {}", id);

        let path = self.path.join(id);
        if !path.exists() {
            return Err(anyhow::anyhow!("Image {} not found", id));
        }

        let image = Image {
            id: id.to_string(),
            name: id.to_string(),
            path,
        };

        trace!("find_disk {:?}", image);
        Ok(image)
    }

    async fn find_kernel(&self, id: &str) -> Result<Image, Error> {
        self.find_disk(id).await
    }

    async fn find_rootfs(&self, id: &str) -> Result<Image, Error> {
        self.find_disk(id).await
    }
}
