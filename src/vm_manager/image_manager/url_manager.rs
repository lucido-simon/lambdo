use std::path::PathBuf;

use anyhow::Error;
use futures::StreamExt;
use tracing::debug;
use tracing::info;
use tracing::trace;

use super::{Image, ImageManager, ImageManifest};

pub struct UrlImageManager {
    pub cache: PathBuf,
}

impl UrlImageManager {
    pub fn new(cache: String) -> Self {
        Self {
            cache: cache.into(),
        }
    }

    async fn find_in_cache(&self, image: &ImageManifest) -> Option<Image> {
        let path = self.cache.join(image.id.clone());

        if path.exists() {
            Some(Image {
                id: image.id.to_string(),
                path,
            })
        } else {
            None
        }
    }

    async fn download_image(&self, image: &ImageManifest) -> Result<Image, Error> {
        info!("Downloading image {} from {}", image.id, image.location);

        let path = self.cache.join(image.id.clone());

        let client = reqwest::Client::new();
        let response = client.get(image.location.clone()).send().await?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!(
                "Failed to download image {}: {}",
                image.id,
                response.status(),
            ));
        }

        let content_length = response.content_length();
        let step = content_length.unwrap_or(10_000_000) / 20;

        if let Some(content_length) = content_length {
            info!("Content length: {}", content_length);
        } else {
            info!("No content length");
        }
        trace!("Step: {}", step);

        let mut file = tokio::fs::File::create(path.clone().with_extension(".download")).await?;
        let mut byte_stream = response.bytes_stream();

        let mut read = 0;

        while let Some(item) = byte_stream.next().await {
            let item = item?;

            if (read as u64 / step) != ((read + item.len()) as u64 / step) {
                info!(
                    "Read {} MB of {} MB",
                    read / 1000000,
                    content_length.map_or("unknown".to_string(), |x| (x / 1000000).to_string())
                );
            }

            read += item.len();

            tokio::io::copy(&mut item.as_ref(), &mut file).await?;
        }

        tokio::fs::rename(path.with_extension(".download"), &path).await?;

        info!("Downloaded image {} to {}", image.id, path.display());

        Ok(Image {
            id: image.id.to_string(),
            path,
        })
    }
}

#[async_trait::async_trait]
impl ImageManager for UrlImageManager {
    async fn find_disk(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        trace!("find_disk {}, {}", manifest.id, manifest.location);

        if let Some(image) = self.find_in_cache(manifest).await {
            debug!("Found image {} in cache", image.id);
            return Ok(image);
        } else {
            self.download_image(manifest).await
        }
    }

    async fn find_kernel(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        self.find_disk(manifest).await
    }

    async fn find_rootfs(&self, manifest: &ImageManifest) -> Result<Image, Error> {
        self.find_disk(manifest).await
    }
}
