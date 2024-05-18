use std::collections::HashMap;

use crate::{
    config::LambdoConfig,
    vm_manager::{
        image::{Image, ImageManager},
        state::LambdoStateRef,
        BootOptions, DiskOptions, NetworkOptions, SimpleSpawn, VMManager, VMManagerTrait,
        VMOptions, VMOptionsDTO,
    },
};
use mockall::automock;
use tracing::debug;
use uuid::Uuid;

pub use crate::vm_manager::Error;

#[automock]
#[async_trait::async_trait]
pub trait LambdoApiServiceTrait: Send + Sync {
    async fn start(&self, request: VMOptionsDTO) -> Result<(String, HashMap<u16, u16>), Error>;
    async fn stop(&self, id: &str) -> Result<(), Error>;

    async fn simple_spawn(
        &self,
        request: SimpleSpawn,
    ) -> Result<(String, HashMap<u16, u16>), Error>;
}

pub struct LambdoApiService {
    pub config: LambdoConfig,
    pub vm_manager: Box<dyn VMManagerTrait>,
    pub image_manager: Box<dyn ImageManager>,
}

impl LambdoApiService {
    pub async fn new(
        config: LambdoConfig,
        image_manager: Box<dyn ImageManager>,
    ) -> Result<Self, Error> {
        let state = crate::vm_manager::state::LambdoState::new(config.clone());
        let vm_manager =
            VMManager::from_state(std::sync::Arc::new(tokio::sync::Mutex::new(state))).await?;
        Ok(LambdoApiService {
            config,
            vm_manager: Box::new(vm_manager),
            image_manager,
        })
    }

    pub async fn to_options(&self, request: VMOptionsDTO) -> Result<VMOptions, Error> {
        let kernel = self.find_kernel(&request.boot.kernel_image_path).await?;
        let rootfs = if let Some(path) = request.boot.initrd_path {
            Some(self.find_rootfs(&path).await?)
        } else {
            None
        };

        let disks = request.disks.iter().map(|disk| async {
            self.image_manager
                .find_disk(&disk.id)
                .await
                .map(|image| DiskOptions {
                    image,
                    is_readonly: disk.is_readonly,
                    is_root_device: disk.is_root_device,
                })
        });

        let disks = futures::future::try_join_all(disks)
            .await
            .map_err(Error::ImageError)?;

        Ok(VMOptions {
            boot: BootOptions {
                kernel,
                initrd: rootfs,
                boot_args: request.boot.boot_args,
            },
            disks,
            network: request.network,
        })
    }

    pub async fn new_with_state(
        state: LambdoStateRef,
        image_manager: Box<dyn ImageManager>,
    ) -> Result<Self, Error> {
        let config = state.lock().await.config.clone();
        let vm_manager = VMManager::from_state(state).await?;
        Ok(LambdoApiService {
            config,
            vm_manager: Box::new(vm_manager),
            image_manager,
        })
    }

    async fn find_kernel(&self, kernel: &str) -> Result<Image, Error> {
        self.image_manager
            .find_kernel(kernel)
            .await
            .map_err(Error::ImageError)
    }

    async fn find_rootfs(&self, rootfs: &str) -> Result<Image, Error> {
        self.image_manager
            .find_rootfs(rootfs)
            .await
            .map_err(Error::ImageError)
    }
}

#[async_trait::async_trait]
impl LambdoApiServiceTrait for LambdoApiService {
    async fn start(&self, request: VMOptionsDTO) -> Result<(String, HashMap<u16, u16>), Error> {
        let id = Uuid::new_v4().to_string();
        let options = self.to_options(request).await?;

        let response = match self
            .vm_manager
            .start_vm(options)
            .await
            .map(|id| async move {
                let ports = self.vm_manager.get_used_ports_of_vm(&id).await;
                (id, ports.unwrap_or_default())
            }) {
            Ok(response) => Ok(response.await),
            Err(e) => Err(e),
        };

        debug!("VM started with id: {}", id);

        response
    }

    async fn stop(&self, id: &str) -> Result<(), Error> {
        self.vm_manager.stop_vm(id).await.map(|_| ())
    }

    async fn simple_spawn(
        &self,
        request: SimpleSpawn,
    ) -> Result<(String, HashMap<u16, u16>), Error> {
        let id = Uuid::new_v4().to_string();
        let used_ports = self.vm_manager.get_used_ports().await;

        let port_mapping = request
            .requested_ports
            .iter()
            .map(|guest| {
                for i in 10000_u16..20000 {
                    if !used_ports.contains(&i) {
                        return Ok((i, *guest));
                    }
                }
                Err(Error::NetSetupError(anyhow::anyhow!("No free port found")))
            })
            .collect::<Result<Vec<(u16, u16)>, Error>>()?;

        let options = VMOptions {
            boot: BootOptions {
                kernel: self.find_kernel("vmlinux").await?,
                initrd: None,
                boot_args: None,
            },
            disks: vec![DiskOptions {
                image: self.find_rootfs(&request.rootfs).await?,
                is_readonly: false,
                is_root_device: true,
            }],
            network: NetworkOptions { port_mapping },
        };

        let response = match self
            .vm_manager
            .start_vm(options)
            .await
            .map(|id| async move {
                let ports = self.vm_manager.get_used_ports_of_vm(&id).await;
                (id, ports.unwrap_or_default())
            }) {
            Ok(response) => Ok(response.await),
            Err(e) => Err(e),
        };

        debug!("VM started with id: {}", id);

        response
    }
}
