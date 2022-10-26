use tokio::net::UnixDatagram;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use tokio::sync::Mutex;
use std::sync::Arc;
use p4rs::{TableEntry, Pipeline};
use slog::Logger;

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
    DumpResponse(BTreeMap<u32, Vec<TableEntry>>),
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableAdd {
    pub table: u32,
    pub action: u32,
    pub keyset_data: Vec<u8>,
    pub parameter_data: Vec<u8>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableRemove {
    pub table: u32,
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
                tm.table,
                tm.action,
                &tm.keyset_data,
                &tm.parameter_data,
            );
        }
        ManagementRequest::TableRemove(tm) => {
            pl.remove_table_entry(tm.table, &tm.keyset_data);
        }
        ManagementRequest::RadixRequest => {
            let response = ManagementResponse::RadixResponse(radix as u16);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
        ManagementRequest::DumpRequest => {
            let mut result = BTreeMap::new();

            for i in 0..pl.get_table_count() {
                let entries = match pl.get_table_entries(i) {
                    Some(entries) => entries,
                    None => Vec::new(),
                };
                result.insert(i, entries);
            }

            let response = ManagementResponse::DumpResponse(result);
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
        }
    }

}
