mod net;

use std::path::PathBuf;
use std::{error::Error as STDError, fmt::Display};

use firepilot::builder::drive::DriveBuilder;
use firepilot::builder::executor::FirecrackerExecutorBuilder;
use firepilot::builder::kernel::KernelBuilder;
use firepilot::builder::network_interface::NetworkInterfaceBuilder;
use firepilot::machine::Machine;
use tracing::{debug, error, info, trace};
use uuid::Uuid;

use crate::vm_manager::state::VMState;

use super::state::LambdoState;
use super::VMOptions;
use firepilot::builder::{Builder, Configuration};
use firepilot::{builder, machine};

const DEFAULT_BOOT_ARGS: &str = "console=ttyS0 reboot=k panic=1 pci=off nomodule";

#[derive(Clone, Debug)]
struct VMOptionsWrapper(VMOptions);

impl From<VMOptions> for VMOptionsWrapper {
    fn from(opts: VMOptions) -> Self {
        VMOptionsWrapper(opts)
    }
}

impl TryInto<Configuration> for VMOptionsWrapper {
    type Error = Error;

    fn try_into(self) -> Result<Configuration, Error> {
        let uuid = Uuid::new_v4().to_string();
        let opts = &self.0;
        let mut configuration = Configuration::new(uuid);

        for d in opts.disks.clone().into_iter() {
            debug!("Adding disk {:?}", d);
            let mut drive = DriveBuilder::new();

            drive.path_on_host = Some(d.image.path.canonicalize().map_err(|e| {
                Error::ImageError(anyhow::anyhow!(
                    "Error while getting canonical path: {:?}",
                    e
                ))
            })?);
            drive.drive_id = Some(d.image.id);
            drive.is_read_only = d.is_readonly;
            drive.is_root_device = d.is_root_device;

            debug!("Drive {:?}", drive);

            configuration = configuration.with_drive(drive.try_build().map_err(Error::VmmNew)?);
        }

        let mut kernel = KernelBuilder::new();

        kernel.boot_args = Some(
            opts.boot
                .boot_args
                .clone()
                .unwrap_or(DEFAULT_BOOT_ARGS.to_string()),
        );
        kernel.initrd_path = if let Some(initrd) = opts.boot.initrd.clone() {
            Some(initrd.path.into_os_string().into_string().map_err(|e| {
                Error::ImageError(anyhow::anyhow!(
                    "String manipulation error for path {}",
                    e.to_string_lossy()
                ))
            })?)
        } else {
            None
        };

        kernel.kernel_image_path = Some(
            opts.boot
                .kernel
                .path
                .clone()
                .into_os_string()
                .into_string()
                .map_err(|e| {
                    Error::ImageError(anyhow::anyhow!(
                        "String manipulation error for path {}",
                        e.to_string_lossy()
                    ))
                })?,
        );

        trace!("Kernel {:?}", kernel);

        let network = NetworkInterfaceBuilder::new()
            .with_host_dev_name("lambdo0".to_string())
            .with_iface_id("tap0".to_string())
            .try_build()
            .map_err(Error::VmmNew)?;

        let executor = FirecrackerExecutorBuilder::new()
            .with_chroot("/tmp".to_string())
            .with_exec_binary(PathBuf::from("/usr/bin/firecracker"))
            .try_build()
            .map_err(Error::VmmNew)?;

        configuration = configuration
            .with_kernel(kernel.try_build().unwrap())
            .with_executor(executor)
            .with_interface(network);

        Ok(configuration)
    }
}

#[derive(Debug)]
pub enum Error {
    VmmNew(builder::BuilderError),
    VmmConfigure(machine::FirepilotError),
    VmmRun(machine::FirepilotError),
    ImageError(anyhow::Error),
    Other(anyhow::Error),
    NetSetupError(anyhow::Error),
    NoIPAvailable,
    VmNotFound,
    VmAlreadyEnded,
}

impl STDError for Error {}

impl Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::VmmNew(e) => write!(f, "Error while creating VMM: {:?}", e),
            Error::VmmConfigure(e) => write!(f, "Error while configuring VMM: {:?}", e),
            Error::VmmRun(e) => write!(f, "Error while running VMM: {:?}", e),
            Error::ImageError(e) => write!(f, "Error with images: {:?}", e),
            Error::Other(e) => write!(f, "Other error: {:?}", e),
            Error::NetSetupError(e) => write!(f, "Error while setting up network: {:?}", e),
            Error::NoIPAvailable => write!(f, "No IP address available"),
            Error::VmNotFound => write!(f, "VM not found"),
            Error::VmAlreadyEnded => write!(f, "VM already ended"),
        }
    }
}

pub async fn start(state: &mut LambdoState, vm_options: VMOptions) -> Result<String, Error> {
    trace!("Creating VMState");
    let mut configuration: Configuration = VMOptionsWrapper::from(vm_options.clone()).try_into()?;
    let mut configuration_cloned: Configuration =
        VMOptionsWrapper::from(vm_options.clone()).try_into()?;

    let id = configuration.vm_id.clone();

    let ip = net::find_available_ip(state).await.map_err(|e| {
        error!("Error while finding available IP address: {:?}", e);
        Error::NoIPAvailable
    })?;

    info!("Creating tap device");
    let tap_name = net::create_tap_device(&id).await.map_err(|e| {
        error!("Error while creating tap device: {:?}", e);
        Error::NetSetupError(e)
    })?;

    configuration.interfaces[0]
        .host_dev_name
        .clone_from(&tap_name);

    let mut vm_state = VMState::new(configuration);
    vm_state.port_mapping = vm_options.network.port_mapping.into_iter().collect();

    vm_state.ip = Some(ip);

    debug!("Adding interface to bridge");

    net::add_interface_to_bridge(&tap_name, &*state).map_err(|e| {
        error!("Error while adding interface to bridge: {:?}", e);
        Error::NoIPAvailable
    })?;

    net::add_boot_option(&mut vm_state, state).map_err(|e| {
        error!("Error while adding boot option: {:?}", e);
        Error::NetSetupError(e)
    })?;

    debug!("Adding port mapping");
    trace!("Port mapping: {:?}", vm_state.port_mapping);
    net::create_port_mapping(&mut vm_state, state).map_err(|e| {
        error!("Error while adding port mapping: {:?}", e);
        Error::NetSetupError(e)
    })?;

    configuration_cloned.interfaces[0] = vm_state.configuration.interfaces[0].clone();
    configuration_cloned
        .kernel
        .clone_from(&vm_state.configuration.kernel);

    let mut machine = Machine::new();
    machine.create(configuration_cloned).await.map_err(|e| {
        error!("Error while creating VMM: {:?}", e);
        Error::VmmConfigure(e)
    })?;

    info!("Starting execution for {:?}", vm_state);

    machine.start().await.unwrap();
    vm_state.machine = Some(machine);

    state.vms.push(vm_state);

    Ok(id)
}

pub async fn stop(state: &mut LambdoState, id: &str) -> Result<(), Error> {
    debug!("Stopping VM {}", id);

    let vm_index = state
        .vms
        .iter()
        .position(|vm| vm.configuration.vm_id == id)
        .ok_or(Error::VmNotFound)?;

    let mut vm = state.vms.remove(vm_index);

    let res = vm
        .machine
        .as_mut()
        .ok_or(Error::Other(anyhow::anyhow!("VM is not running")))?
        .stop()
        .await
        .map_err(|e| {
            error!("Error while stopping VM: {:?}", e);
            Error::Other(anyhow::anyhow!("Error while stopping VM: {:?}", e))
        });

    match cleanup_network(state, &mut vm).await {
        Ok(()) => res,
        Err(e) => {
            error!("Error while cleaning up network: {:?}", e);
            if res.is_err() {
                res
            } else {
                Err(e)
            }
        }
    }
}

pub async fn cleanup_network(state: &mut LambdoState, vm: &mut VMState) -> Result<(), Error> {
    debug!(
        "Cleaning up VM Network configuration for {} ",
        vm.configuration.vm_id
    );

    let ip = vm
        .ip
        .as_ref()
        .ok_or(Error::Other(anyhow::anyhow!("VM has no IP address")))?;

    net::remove_port_mapping(&vm.port_mapping, ip).map_err(|e| {
        error!("Error while removing port mapping: {:?}", e);
        Error::NetSetupError(e)
    })?;

    let tap_name = vm.configuration.interfaces[0].host_dev_name.clone();

    debug!(
        "Removing interface {} from bridge {}",
        tap_name, state.config.api.network.bridge
    );

    net::remove_interface_from_bridge(&tap_name, &state.config.api.network.bridge).map_err(
        |e| {
            error!("Error while removing tap device: {:?}", e);
            Error::NetSetupError(e)
        },
    )?;

    debug!("Removing tap device {}", tap_name);

    net::remove_tap_device(&tap_name).await.map_err(|e| {
        error!("Error while removing tap device: {:?}", e);
        Error::NetSetupError(e)
    })?;

    Ok(())
}
