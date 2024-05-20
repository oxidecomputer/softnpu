// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

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
