// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use p4rs::{Pipeline, TableEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Mutex;
use tokio::net::UnixDatagram;

// Re-export p4rs so consumers can rely on matching types
pub use p4rs;

#[derive(Debug, Default, Serialize, Deserialize)]
pub enum ManagementRequest {
    #[default]
    RadixRequest,
    TableAdd(TableAdd),
    TableRemove(TableRemove),
    DumpRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ManagementResponse {
    RadixResponse(u16),
    DumpResponse(BTreeMap<String, Vec<TableEntry>>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableAdd {
    pub table: String,
    pub action: String,
    pub keyset_data: Vec<u8>,
    pub parameter_data: Vec<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableRemove {
    pub table: String,
    pub keyset_data: Vec<u8>,
}

pub async fn handle_management_message(
    msg: ManagementRequest,
    pipeline: &Mutex<Box<dyn Pipeline>>,
    uds: &UnixDatagram,
    uds_dst: &Path,
    radix: usize,
) {
    match msg {
        ManagementRequest::TableAdd(tm) => {
            let mut pl = pipeline.lock().unwrap();
            pl.add_table_entry(
                &tm.table,
                &tm.action,
                &tm.keyset_data,
                &tm.parameter_data,
                0,
            );
        }
        ManagementRequest::TableRemove(tm) => {
            let mut pl = pipeline.lock().unwrap();
            pl.remove_table_entry(&tm.table, &tm.keyset_data);
        }
        ManagementRequest::RadixRequest => {
            let response = ManagementResponse::RadixResponse(radix as u16);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
        ManagementRequest::DumpRequest => {
            let result = {
                let pl = pipeline.lock().unwrap();
                pl.get_table_ids()
                    .into_iter()
                    .map(|id| {
                        let entries =
                            pl.get_table_entries(id).unwrap_or_default();
                        (id.to_string(), entries)
                    })
                    .collect::<BTreeMap<String, Vec<TableEntry>>>()
            };

            let response = ManagementResponse::DumpResponse(result);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
    }
}
