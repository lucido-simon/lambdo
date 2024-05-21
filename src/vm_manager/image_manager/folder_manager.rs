use std::path::PathBuf;

use anyhow::Error;
use tracing::trace;

use super::{Image, ImageManager, ImageManifest};

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
    async fn find_disk(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        trace!("find_disk {}, {}", manifest.id, manifest.location);

        let path = self.path.join(manifest.location.clone());
        if !path.exists() {
            return Err(anyhow::anyhow!(
                "Image {} ({}) not found",
                manifest.id,
                path.display()
            ));
        }

        let image = Image {
            id: manifest.id.to_string(),
            path,
        };

        trace!("find_disk {:?}", image);
        Ok(image)
    }

    async fn find_kernel(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        self.find_disk(manifest).await
    }

    async fn find_rootfs(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        self.find_disk(manifest).await
    }
}
