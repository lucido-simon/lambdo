pub mod service;

use actix_web::{post, web, Responder};
use serde::{Deserialize, Serialize};
use tracing::{debug, error, info};

use crate::{
    api::service::{LambdoApiService, LambdoApiServiceTrait},
    vm_manager::{SimpleSpawn, VMOptionsDTO},
};

use std::{collections::HashMap, error::Error as STDError};

#[derive(Serialize, Deserialize)]
pub struct StartResponse {
    pub id: String,
    pub port_mapping: Vec<(u16, u16)>,
}

impl From<(String, HashMap<u16, u16>)> for StartResponse {
    fn from(value: (String, HashMap<u16, u16>)) -> Self {
        let (id, port_mapping) = value;
        let port_mapping = port_mapping.into_iter().collect();
        StartResponse { id, port_mapping }
    }
}

#[post("/start")]
pub async fn start_route(
    vm_options: web::Json<VMOptionsDTO>,
    api_service: web::Data<LambdoApiService>,
) -> Result<impl Responder, Box<dyn STDError>> {
    debug!("Received HTTP VM Start request body: {:?}", vm_options);

    let service = api_service.get_ref();
    let result = service.start(vm_options.into_inner()).await;

    if let Ok(result) = result.as_ref() {
        info!("VM started with id: {}", result.0);
    } else {
        error!("Error while starting VM: {:?}", result);
    }

    let response = result?;

    Ok(web::Json(StartResponse::from(response)))
}

#[post("/spawn")]
pub async fn simple_spawn_route(
    vm_options: web::Json<SimpleSpawn>,
    api_service: web::Data<LambdoApiService>,
) -> Result<impl Responder, Box<dyn STDError>> {
    debug!("Received HTTP VM Start request body: {:?}", vm_options);

    let service = api_service.get_ref();
    let result = service.simple_spawn(vm_options.into_inner()).await;

    if let Ok(result) = result.as_ref() {
        info!("VM started with id: {}", result.0);
    } else {
        error!("Error while starting VM: {:?}", result);
    }

    let response = result?;

    Ok(web::Json(StartResponse::from(response)))
}
