use std::collections::HashMap;
use std::process::Command;
use std::str::FromStr;

use anyhow::anyhow;
use anyhow::Result;
use cidr::Ipv4Inet;
use tracing::{debug, info, trace};

use crate::vm_manager::state::LambdoState;
use crate::vm_manager::state::VMState;
use crate::vm_manager::state::VMStatus;

pub(super) fn add_interface_to_bridge(interface_name: &String, state: &LambdoState) -> Result<()> {
    let bridge_name = &state.config.api.network.bridge;
    debug!(
        "adding interface {} to bridge {}",
        interface_name, bridge_name
    );

    trace!("fetching interface id");
    let interface_id = network_bridge::interface_id(interface_name)
        .map_err(|e| anyhow!("error when fetching interface id: {}", e))?;

    trace!("interface id: {}", interface_id);
    network_bridge::add_interface_to_bridge(interface_id, bridge_name)
        .map_err(|e| anyhow!("error when adding interface to bridge: {}", e))?;

    debug!("bringing up interface");
    Command::new("ip")
        .args(["link", "set", interface_name, "up"])
        .output()
        .map_err(|e| anyhow!("error when bringing up interface: {}", e))?;

    info!(
        "interface {} added to bridge {}",
        interface_name, bridge_name
    );
    Ok(())
}

pub(super) async fn create_tap_device(id: &str) -> Result<String> {
    let truncated_id = id[..8].to_string();
    let tap_name = format!("tap-{}", truncated_id);
    let tap = tokio_tun::TunBuilder::new()
        .name(&tap_name)
        .tap(true)
        .packet_info(false)
        .persist()
        .up()
        .try_build();

    tap.map_err(|e| anyhow!("error when creating tap device: {}", e))?;
    Ok(tap_name)
}

pub(super) async fn find_available_ip(state: &LambdoState) -> Result<Ipv4Inet> {
    let config = &state.config;
    // Safe since we checked the validity of the address before
    let host_ip = Ipv4Inet::from_str(&config.api.network.bridge_address).unwrap();

    let used_ip: &Vec<_> = &state
        .vms
        .iter()
        .filter_map(|vm| {
            debug!("VM {:?} has ip {:?}", vm.configuration.vm_id, vm.ip);
            match vm.ip {
                Some(ip)
                    if vm.get_state() != VMStatus::Exited
                        || vm.get_state() != VMStatus::Terminated =>
                {
                    Some(ip.address())
                }
                _ => None,
            }
        })
        .collect();

    debug!("looking for available ip in {}", host_ip);
    trace!("used ip: {:?}", used_ip);
    let mut ip = host_ip;
    ip.increment();

    while used_ip.contains(&ip.address()) {
        trace!("ip {} is already used, trying next one", ip);
        if ip.increment() {
            // return Err(anyhow!("no available ip"));
        }
    }

    info!("found available ip: {}", ip);
    Ok(ip)
}

pub(super) fn add_boot_option(vm: &mut VMState, state: &LambdoState) -> Result<()> {
    debug!("adding network boot option to kernel");
    let mut boot_args = vm
        .configuration
        .kernel
        .as_ref()
        .ok_or(anyhow!("Boot source not configured"))?
        .boot_args
        .clone()
        .unwrap_or_default();

    let guest_ip = vm.ip.ok_or(anyhow!("IP not set"))?;
    let netmask = guest_ip.mask();
    let gateway = state
        .config
        .api
        .network
        .bridge_address
        .split('/')
        .next()
        .unwrap_or_default();

    debug!("guest ip: {}", guest_ip);
    debug!("gateway: {}", gateway);
    debug!("netmask: {}", netmask);

    boot_args.push_str(
        format!(
            " ip={}::{}:{}::eth0:on",
            guest_ip.address(),
            gateway,
            netmask
        )
        .as_str(),
    );

    debug!("boot args: {}", boot_args);

    vm.configuration.kernel.as_mut().unwrap().boot_args = Some(boot_args);

    Ok(())
}

pub(super) fn create_port_mapping(
    vm_state: &mut VMState,
    lambdo_state: &LambdoState,
) -> Result<()> {
    for (host_port, guest_port) in vm_state.port_mapping.iter() {
        for vm in &lambdo_state.vms {
            if vm.get_state() == VMStatus::Running
                || vm.get_state() == VMStatus::Pending && vm.port_mapping.contains_key(host_port)
            {
                return Err(anyhow!("Port mapping already exists for {}", host_port));
            }
        }

        let ip_table =
            iptables::new(false).map_err(|e| anyhow!("error when creating nat table: {}", e))?;

        debug!("adding port mapping for {} to {}", host_port, guest_port);

        // PORT MAPPING
        ip_table
            .append(
                "nat",
                "PREROUTING",
                format!(
                    "-p tcp --dport {} -j DNAT --to-destination {}:{}",
                    host_port,
                    vm_state.ip.ok_or(anyhow!("IP not set"))?.address(),
                    guest_port
                )
                .as_str(),
            )
            .map_err(|e| anyhow!("error when adding port mapping: {}", e))?;

        //MASQUERADE
        ip_table
            .append(
                "nat",
                "POSTROUTING",
                format!(
                    "-p tcp -d {} --dport {} -j MASQUERADE",
                    vm_state.ip.ok_or(anyhow!("IP not set"))?.address(),
                    guest_port
                )
                .as_str(),
            )
            .map_err(|e| anyhow!("error when adding port mapping: {}", e))?;

        //ACCEPT FORWARD
        ip_table
            .append(
                "filter",
                "FORWARD",
                format!(
                    "-p tcp -d {} --dport {} -m state --state NEW,ESTABLISHED,RELATED -j ACCEPT",
                    vm_state.ip.ok_or(anyhow!("IP not set"))?.address(),
                    guest_port
                )
                .as_str(),
            )
            .map_err(|e| anyhow!("error when adding port mapping: {}", e))?;
    }

    Ok(())
}

pub(super) fn remove_port_mapping(
    port_mapping: &HashMap<u16, u16>,
    vm_ip: &Ipv4Inet,
) -> Result<()> {
    debug!("removing port mapping");
    trace!("port mapping: {:?}", port_mapping);
    trace!("vm ip: {}", vm_ip);

    let ip_table =
        iptables::new(false).map_err(|e| anyhow!("error when creating nat table: {}", e))?;

    let address = vm_ip.address();

    for (host_port, guest_port) in port_mapping {
        ip_table
            .delete(
                "nat",
                "PREROUTING",
                format!(
                    "-p tcp --dport {} -j DNAT --to-destination {}:{}",
                    host_port, address, guest_port
                )
                .as_str(),
            )
            .map_err(|e| anyhow!("error when removing port mapping: {}", e))?;

        ip_table
            .delete(
                "nat",
                "POSTROUTING",
                format!("-p tcp -d {} --dport {} -j MASQUERADE", address, guest_port).as_str(),
            )
            .map_err(|e| anyhow!("error when removing port mapping: {}", e))?;

        ip_table
            .delete(
                "filter",
                "FORWARD",
                format!(
                    "-p tcp -d {} --dport {} -m state --state NEW,ESTABLISHED,RELATED -j ACCEPT",
                    address, guest_port
                )
                .as_str(),
            )
            .map_err(|e| anyhow!("error when removing port mapping: {}", e))?;
    }

    Ok(())
}

pub(super) fn remove_interface_from_bridge(interface_name: &str, bridge_name: &str) -> Result<()> {
    let interface_id = network_bridge::interface_id(interface_name)
        .map_err(|e| anyhow!("error when fetching interface id: {}", e))?;

    network_bridge::delete_interface_from_bridge(interface_id, bridge_name)
        .map_err(|e| anyhow!("error when removing interface from bridge: {}", e))?;

    Ok(())
}

pub(super) async fn remove_tap_device(tap_name: &String) -> Result<()> {
    tokio::process::Command::new("ip")
        .args(["link", "delete", tap_name])
        .output()
        .await
        .map_err(|e| anyhow!("error when removing tap device: {}", e))?;

    Ok(())
}
