pub mod api;
pub mod config;
pub mod model;
pub mod vm_manager;

use std::sync::Arc;

use config::{ImageManagerStrategy, LambdoConfig};
use thiserror::Error;
use tracing_subscriber::EnvFilter;

use crate::{
    api::{service::LambdoApiService, simple_spawn_route, start_route, stop_route},
    vm_manager::{
        image_manager::{
            folder_manager::FolderImageManager, url_manager::UrlImageManager, ImageManager,
        },
        state::LambdoState,
    },
};
use actix_web::{web, App, HttpServer};
use clap::Parser;
use tokio::sync::Mutex;
use tracing::{debug, error, info, trace};

#[derive(Parser)]
#[clap(
    version = "0.1",
    author = "Polytech Montpellier - DevOps",
    about = "A Serverless runtime in Rust"
)]
pub struct LambdoOpts {
    /// Config file path
    #[clap(short, long, default_value = "/etc/lambdo/config.yaml")]
    config: String,
}

#[derive(Error, Debug)]
pub enum LambdoError {
    #[error(transparent)]
    Other(#[from] anyhow::Error),
    #[error("unknown lambdo error")]
    Unknown,
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    tracing_subscriber::fmt::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .init();

    info!("starting up ...");

    let options = LambdoOpts::parse();

    debug!("loading config file at {}", options.config);
    let config = LambdoConfig::load(options.config.as_str()).unwrap();
    trace!(
        "config file loaded successfully with content: {:#?}",
        config
    );

    info!("setting up");
    let lambdo_state = Arc::new(Mutex::new(LambdoState::new(config.clone())));

    let image_manager: Box<dyn ImageManager> = match config.api.image_manager.strategy {
        ImageManagerStrategy::Folder => Box::new(FolderImageManager::new(
            config.api.image_manager.images_folder,
        )),
        ImageManagerStrategy::Url => {
            Box::new(UrlImageManager::new(config.api.image_manager.images_folder))
        }
    };

    let api_service = LambdoApiService::new_with_state(lambdo_state, image_manager)
        .await
        .map_err(|e| {
            error!("failed to set up API service: {}", e);
        })
        .unwrap();

    info!("everything is set up, starting servers");

    let http_host = &config.api.network.web_host;
    let http_port = config.api.network.web_port;
    let app_state = web::Data::new(api_service);
    info!("Starting web server on {}:{}", http_host, http_port);
    HttpServer::new(move || {
        App::new()
            .app_data(app_state.clone())
            .service(start_route)
            .service(simple_spawn_route)
            .service(stop_route)
    })
    .bind((http_host.clone(), http_port))?
    .run()
    .await
}
