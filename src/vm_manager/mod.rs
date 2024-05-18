pub mod state;
use mockall::automock;
use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

pub use vmm::Error;

use anyhow::anyhow;

use std::{collections::HashMap, net::IpAddr, str::FromStr};
use tracing::{debug, error, info, trace};

use self::{
    image::Image,
    state::LambdoStateRef,
    vmm::{start, stop},
};

pub mod image;
mod vmm;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SimpleSpawn {
    pub rootfs: String,
    #[serde(rename = "requestedPorts")]
    pub requested_ports: Vec<u16>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootOptionsDTO {
    /// Kernel boot arguments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_args: Option<String>,
    /// Host level path to the initrd image used to boot the guest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initrd_path: Option<String>,
    /// Host level path to the kernel image used to boot the guest
    pub kernel_image_path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BootOptions {
    /// Kernel boot arguments
    #[serde(skip_serializing_if = "Option::is_none")]
    pub boot_args: Option<String>,
    /// Host level path to the initrd image used to boot the guest
    #[serde(skip_serializing_if = "Option::is_none")]
    pub initrd: Option<Image>,
    /// Host level path to the kernel image used to boot the guest
    pub kernel: Image,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiskOptionsDTO {
    pub id: String,
    pub is_readonly: bool,
    pub is_root_device: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DiskOptions {
    pub image: Image,
    pub is_readonly: bool,
    pub is_root_device: bool,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VMOptionsDTO {
    pub boot: BootOptionsDTO,
    pub disks: Vec<DiskOptionsDTO>,
    pub network: NetworkOptions,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VMOptions {
    pub boot: BootOptions,
    pub disks: Vec<DiskOptions>,
    pub network: NetworkOptions,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkOptions {
    #[serde(default)]
    pub port_mapping: Vec<(u16, u16)>,
}

#[automock]
#[async_trait::async_trait]
pub trait VMManagerTrait: Sync + Send {
    async fn from_state(state: LambdoStateRef) -> Result<Self, Error>
    where
        Self: Sized;

    async fn start_vm(&self, request: VMOptions) -> Result<String, Error>;
    async fn stop_vm(&self, id: &str) -> Result<(), Error>;
    async fn get_used_ports(&self) -> Vec<u16>;
    async fn get_used_ports_of_vm(&self, vm_id: &str) -> Option<HashMap<u16, u16>>;
}

pub struct VMManager {
    pub state: LambdoStateRef,
}

#[async_trait::async_trait]
impl VMManagerTrait for VMManager {
    async fn from_state(state: LambdoStateRef) -> Result<Self, Error> {
        let vmm_manager = VMManager { state };

        {
            let state = vmm_manager.state.lock().await;
            setup_bridge(&state).await.map_err(|e| {
                error!("Error while setting up bridge: {:?}", e);
                Error::NetSetupError(e)
            })?;
        }

        Ok(vmm_manager)
    }

    async fn start_vm(&self, request: VMOptions) -> Result<String, Error> {
        let mut state = self.state.lock().await;

        debug!("Creating VM with option {:?}", request);

        let id = start(&mut state, request).await.map_err(|e| {
            error!("Error while running VM: {:?}", e);
            e
        })?;

        info!("Waiting for a connection from VMM {}", id);

        drop(state);

        state = self.state.lock().await;
        let vm = state
            .vms
            .iter_mut()
            .find(|vm| vm.configuration.vm_id == id)
            .unwrap();

        Ok(vm.configuration.vm_id.clone())
    }

    async fn stop_vm(&self, id: &str) -> Result<(), Error> {
        debug!("Stopping VM {}", id);
        let mut state = self.state.lock().await;

        stop(&mut state, id).await.map_err(|e| {
            error!("Error while stopping VM: {:?}", e);
            e
        })?;

        Ok(())
    }

    async fn get_used_ports(&self) -> Vec<u16> {
        let state = self.state.lock().await;
        state
            .vms
            .iter()
            .flat_map(|vm| vm.port_mapping.keys())
            .cloned()
            .collect()
    }

    async fn get_used_ports_of_vm(&self, vm_id: &str) -> Option<HashMap<u16, u16>> {
        let state = self.state.lock().await;
        let vm = state.vms.iter().find(|vm| vm.configuration.vm_id == vm_id);
        vm.map(|vm| vm.port_mapping.clone())
    }
}

async fn setup_bridge(state: &state::LambdoState) -> anyhow::Result<()> {
    let config = &state.config;
    let bridge_name = &config.api.bridge;
    let bridge_address = &config.api.bridge_address;
    trace!("validating bridge address");
    let bridge_address = cidr::Ipv4Inet::from_str(bridge_address)
        .map_err(|e| anyhow!("invalid bridge address: {}", e))?;
    trace!("bridge address is valid");
    trace!("validating bridge name");
    if bridge_name.len() > 15 {
        return Err(anyhow!("bridge name is too long"));
    }
    trace!("bridge name is valid");

    info!(
        "setting up bridge {} with address {}",
        bridge_name, bridge_address
    );
    let (bridge, interface_exists) = network_bridge::interface_id(bridge_name)
        .map_or_else(
            |e| {
                trace!("error when fetching bridge id: {}", e);
                debug!("bridge {} does not exist, creating it", bridge_name);
                network_bridge::create_bridge(bridge_name).map(|id| (id, false))
            },
            |id| {
                debug!("bridge {} already exists, using it", bridge_name);
                Ok((id, true))
            },
        )
        .map_err(|e| {
            error!("error when creating bridge, am I running as root?");
            anyhow!("error when creating bridge: {}", e)
        })?;

    trace!("bridge id: {}", bridge);
    debug!("looking for existing bridge address");
    let addresses = NetworkInterface::show()
        .map_err(|e| anyhow!("error when fetching network interfaces: {}", e))?
        .into_iter()
        .filter(|iface| iface.name == *bridge_name)
        .flat_map(|iface| iface.addr)
        .collect::<Vec<_>>();

    trace!("existing addresses: {:?}", addresses);
    let address_exists = addresses.iter().any(|addr| {
        addr.ip() == bridge_address.address()
            && addr.netmask() == Some(IpAddr::V4(bridge_address.mask()))
    });

    if address_exists {
        debug!("bridge address already exists, skipping");
    } else {
        debug!("bridge address does not exist, creating it");
        trace!(
            "Values: {} {}/{}",
            bridge_name,
            bridge_address.address(),
            bridge_address.network_length()
        );
        Command::new("ip")
            .args([
                "addr",
                "add",
                &format!(
                    "{}/{}",
                    bridge_address.address(),
                    bridge_address.network_length()
                ),
                "dev",
                bridge_name,
            ])
            .output()
            .await
            .map_err(|e| anyhow!("error when adding bridge address: {}", e))?;
    }

    debug!("setting up bridge firewall");

    if !interface_exists || !address_exists {
        let default_interface_name = default_net::interface::get_default_interface_name()
            .ok_or(anyhow!("no default interface found"))?;

        let iptables = iptables::new(false)
            .map_err(|e| anyhow!("error when setting up bridge firewall: {}", e))?;

        iptables
            .append(
                "filter",
                "FORWARD",
                format!("-i {} -o {} -j ACCEPT", default_interface_name, bridge_name).as_str(),
            )
            .map_err(|e| anyhow!("error when setting up bridge firewall: {}", e))?;

        iptables
            .append(
                "filter",
                "FORWARD",
                format!("-i {} -o {} -j ACCEPT", bridge_name, default_interface_name).as_str(),
            )
            .map_err(|e| anyhow!("error when setting up bridge firewall: {}", e))?;

        iptables
            .append(
                "nat",
                "POSTROUTING",
                format!("-o {} -j MASQUERADE", default_interface_name).as_str(),
            )
            .map_err(|e| anyhow!("error when setting up bridge firewall: {}", e))?;
    } else {
        debug!("bridge firewall already set up, skipping");
    }

    debug!("bringing up bridge");

    Command::new("ip")
        .args(["link", "set", bridge_name, "up"])
        .output()
        .await
        .map_err(|e| anyhow!("error when bringing up bridge: {}", e))?;

    info!("bridge {} is ready", bridge_name);
    Ok(())
}
