use std::collections::HashMap;
use std::fmt::Debug;

use cidr::Ipv4Inet;
use tracing::debug;

use crate::{config::LambdoConfig, vm_manager};

pub type LambdoStateRef = std::sync::Arc<tokio::sync::Mutex<LambdoState>>;

pub struct LambdoState {
    pub vms: Vec<VMState>,
    pub config: LambdoConfig,
}

impl LambdoState {
    pub fn new(config: LambdoConfig) -> Self {
        LambdoState {
            vms: Vec::new(),
            config,
        }
    }
}

#[derive(Debug)]
pub struct VMState {
    pub machine: Option<firepilot::machine::Machine>,
    pub configuration: firepilot::builder::Configuration,
    pub status: VMStatus,
    pub ip: Option<Ipv4Inet>,
    pub port_mapping: HashMap<u16, u16>,
}

impl VMState {
    pub fn new(configuration: firepilot::builder::Configuration) -> Self {
        VMState {
            machine: None,
            configuration,
            status: VMStatus::Pending,
            ip: None,
            port_mapping: HashMap::new(),
        }
    }

    pub fn get_state(&self) -> VMStatus {
        self.status
    }

    pub fn get_id(&self) -> String {
        self.configuration.vm_id.clone()
    }

    pub async fn start(&mut self) -> Result<(), anyhow::Error> {
        self.machine
            .as_mut()
            .unwrap()
            .start()
            .await
            .map_err(|e| vm_manager::Error::VmmRun(e).into())
    }

    fn set_state(&mut self, state: VMStatus) {
        match state {
            VMStatus::Pending => {
                debug!("VM {} is pending", self.configuration.vm_id);
                self.status = state;
            }
            VMStatus::Running => {
                debug!("VM {} is running", self.configuration.vm_id);
                self.status = state;
            }
            VMStatus::Exited => {
                debug!("VM {} has exited", self.configuration.vm_id);
                // TODO: Find a way to kill the VM
                // Probably need to make edit lumper
                self.status = state;
            }
            VMStatus::Terminated => {
                debug!("VM {} has terminated", self.configuration.vm_id);
                // TODO: Find a way to kill the VM
                // Probably need to make edit lumper
                self.status = state;
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VMStatus {
    Pending,
    Running,
    Exited,
    Terminated,
}
