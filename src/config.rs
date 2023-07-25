use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
pub struct Port {
    pub sidecar: String,
    pub scrimlet: String,
    pub mtu: usize,
}

#[derive(Default, Debug, Deserialize, Serialize)]
pub struct Config {
    pub p4_program: String,
    pub ports: Vec<Port>,
}
