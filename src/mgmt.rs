// Copyright 2022 Oxide Computer Company

use p4rs::{Pipeline, TableEntry};
use serde::{Deserialize, Serialize};
use slog::Logger;
use std::collections::BTreeMap;
use std::sync::Arc;
use tokio::net::UnixDatagram;
use tokio::sync::Mutex;

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
    pipeline: Arc<Mutex<Box<dyn Pipeline>>>,
    uds: Arc<UnixDatagram>,
    uds_dst: &str,
    radix: usize,
    _log: Logger,
) {
    let mut pl = pipeline.lock().await;

    match msg {
        ManagementRequest::TableAdd(tm) => {
            pl.add_table_entry(
                &tm.table,
                &tm.action,
                &tm.keyset_data,
                &tm.parameter_data,
            );
        }
        ManagementRequest::TableRemove(tm) => {
            pl.remove_table_entry(&tm.table, &tm.keyset_data);
        }
        ManagementRequest::RadixRequest => {
            let response = ManagementResponse::RadixResponse(radix as u16);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
        ManagementRequest::DumpRequest => {
            let mut result = BTreeMap::new();

            for id in pl.get_table_ids() {
                let entries = match pl.get_table_entries(id) {
                    Some(entries) => entries,
                    None => Vec::new(),
                };
                result.insert(id.to_string(), entries);
            }

            let response = ManagementResponse::DumpResponse(result);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
    }
}
