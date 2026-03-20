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

/// Multicast group identifier.
pub type MulticastGroupId = u16;

/// Physical port number within a multicast group.
pub type MulticastPort = u16;

#[derive(Debug, Default, Serialize, Deserialize)]
pub enum ManagementRequest {
    #[default]
    RadixRequest,
    TableAdd(TableAdd),
    TableRemove(TableRemove),
    DumpRequest,
    MulticastGroupCreate(MulticastGroupCreate),
    MulticastGroupRemove(MulticastGroupRemove),
    MulticastPortAdd(MulticastPortAdd),
    MulticastPortRemove(MulticastPortRemove),
    MulticastGroupList,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ManagementResponse {
    RadixResponse(u16),
    DumpResponse(BTreeMap<String, Vec<TableEntry>>),
    MulticastGroupListResponse(BTreeMap<MulticastGroupId, Vec<MulticastPort>>),
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

#[derive(Debug, Serialize, Deserialize)]
pub struct MulticastGroupCreate {
    pub group_id: MulticastGroupId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MulticastGroupRemove {
    pub group_id: MulticastGroupId,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MulticastPortAdd {
    pub group_id: MulticastGroupId,
    pub port: MulticastPort,
    pub rid: u16,
    pub level1_excl_id: u16,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MulticastPortRemove {
    pub group_id: MulticastGroupId,
    pub port: MulticastPort,
}

/// Errors from multicast management operations.
#[derive(Debug, thiserror::Error)]
pub enum MulticastError {
    #[error("group ID 0 is reserved (the runtime treats 0 as no-multicast)")]
    ReservedGroupId,
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
        // Mutating multicast ops are fire-and-forget, matching the
        // TableAdd/TableRemove pattern. Only MulticastGroupList returns
        // a response.
        ManagementRequest::MulticastGroupCreate(req) => {
            if let Err(err) = validate_group_id(req.group_id) {
                eprintln!("MulticastGroupCreate rejected: {err}");
                return;
            }
            let mut pl = pipeline.lock().unwrap();
            pl.add_mcast_group(req.group_id);
        }
        ManagementRequest::MulticastGroupRemove(req) => {
            if let Err(err) = validate_group_id(req.group_id) {
                eprintln!("MulticastGroupRemove rejected: {err}");
                return;
            }
            let mut pl = pipeline.lock().unwrap();
            pl.remove_mcast_group(req.group_id);
        }
        ManagementRequest::MulticastPortAdd(req) => {
            if let Err(err) = validate_group_id(req.group_id) {
                eprintln!("MulticastPortAdd rejected: {err}");
                return;
            }
            // rid and level1_excl_id are Tofino traffic manager concepts
            // for per-replica identification and exclusion. SoftNPU handles
            // these via McastReplicationTag in the codegen instead.
            //
            // Dendrite passes non-zero rid (set to external_group_id) as
            // part of the Tofino replication config, meaning its accepted but
            // unused.
            let mut pl = pipeline.lock().unwrap();
            pl.add_mcast_port(req.group_id, req.port);
        }
        ManagementRequest::MulticastPortRemove(req) => {
            if let Err(err) = validate_group_id(req.group_id) {
                eprintln!("MulticastPortRemove rejected: {err}");
                return;
            }
            let mut pl = pipeline.lock().unwrap();
            pl.remove_mcast_port(req.group_id, req.port);
        }
        ManagementRequest::MulticastGroupList => {
            let result = {
                let pl = pipeline.lock().unwrap();
                pl.get_mcast_groups()
                    .iter()
                    .map(|(&group_id, ports)| {
                        let mut sorted: Vec<MulticastPort> =
                            ports.iter().copied().collect();
                        sorted.sort();
                        (group_id, sorted)
                    })
                    .collect::<BTreeMap<MulticastGroupId, Vec<MulticastPort>>>()
            };
            let response =
                ManagementResponse::MulticastGroupListResponse(result);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
    }
}

fn validate_group_id(group_id: MulticastGroupId) -> Result<(), MulticastError> {
    if group_id == 0 {
        return Err(MulticastError::ReservedGroupId);
    }
    Ok(())
}
