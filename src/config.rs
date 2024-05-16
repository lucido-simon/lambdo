use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, BufReader},
};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum LambdoConfigError {
    #[error("cannot load config file")]
    Load(#[from] io::Error),
    #[error("cannot parse config file")]
    Parse(#[from] serde_yaml::Error),
    #[error("unsupported config kind")]
    KindNotSupported,
    #[error("unsupported config api version")]
    VersionNotSupported,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
#[allow(non_snake_case)]
pub struct LambdoConfig {
    /// The api version of the lambdo config file
    pub apiVersion: String,
    /// The kind of the lambdo config file
    pub kind: String,
    /// The lambdo api configuration
    pub api: LambdoApiConfig,
}

#[derive(Serialize, Deserialize, PartialEq, Debug, Clone)]
pub struct LambdoApiConfig {
    /// Bridge to bind to
    #[serde(default = "default_bridge")]
    pub bridge: String,
    /// Address of the bridge
    #[serde(default = "default_bridge_address")]
    pub bridge_address: String,
    /// The host on which the API server will listen
    pub web_host: String,
    /// The port on which the API server will listen
    pub web_port: u16,
}

fn default_bridge() -> String {
    String::from("lambdo0")
}

fn default_bridge_address() -> String {
    String::from("192.168.10.1/24")
}

impl LambdoConfig {
    /// Load a LambdoConfig from a file.
    ///
    /// Arguments:
    ///
    /// * `path`: The path to the config file.
    ///
    /// Returns:
    ///
    /// A Result<LambdoConfig>
    pub fn load(path: &str) -> Result<Self> {
        let file = File::open(path).map_err(LambdoConfigError::Load)?;
        let reader = BufReader::new(file);
        let config: LambdoConfig =
            serde_yaml::from_reader(reader).map_err(LambdoConfigError::Parse)?;

        if config.kind != "Config" {
            return Err(LambdoConfigError::KindNotSupported.into());
        }

        if config.apiVersion != "lambdo.io/v1alpha1" {
            return Err(LambdoConfigError::VersionNotSupported.into());
        }

        Ok(config)
    }
}
