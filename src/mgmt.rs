// Copyright 2022 Oxide Computer Company

use p4rs::{Pipeline, TableEntry};
use serde::{Deserialize, Serialize};
use slog::Logger;
use std::collections::{BTreeMap, HashMap};
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;
use std::sync::Arc;
use tokio::net::UnixDatagram;
use tokio::sync::Mutex;

const SOFTNPU_UART_PREAMBLE: u8 = 0b11100101;

#[derive(Debug, Default, Serialize, Deserialize)]
pub enum ManagementRequest {
    #[default]
    RadixRequest,
    TableAdd(TableAdd),
    TableRemove(TableRemove),
    TableCounters(TableCounters),
    DumpRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub enum ManagementResponse {
    RadixResponse(u16),
    DumpResponse(BTreeMap<String, Vec<TableEntry>>),
    TableCountersResponse(Option<HashMap<Vec<u8>, u128>>),
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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableCounters {
    pub table: String,
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
                0,
            );
        }
        ManagementRequest::TableRemove(tm) => {
            pl.remove_table_entry(&tm.table, &tm.keyset_data);
        }
        ManagementRequest::TableCounters(tc) => {
            let response = match pl.get_table_counters(&tc.table) {
                None => ManagementResponse::TableCountersResponse(None),
                Some(counters) => {
                    let entries = counters.entries.lock().unwrap().clone();
                    ManagementResponse::TableCountersResponse(Some(entries))
                }
            };
            let buf = serde_json::to_vec(&response).unwrap();
            uds.send_to(&buf, uds_dst).await.unwrap();
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

pub fn dump_tables_propolis() -> BTreeMap<String, Vec<TableEntry>> {
    let mut f = OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty03")
        .unwrap();

    let fd = f.as_raw_fd();
    unsafe {
        let mut term: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(fd, &mut term) != 0 {
            println!("tcgetattr failed, dump may hang");
        }
        term.c_lflag &= !(libc::ICANON | libc::ECHO | libc::ECHOE | libc::ISIG);
        if libc::tcsetattr(fd, libc::TCSANOW, &term) != 0 {
            println!("tcsetattr failed, dump may hang");
        }
    }

    let msg = ManagementRequest::DumpRequest;
    let mut buf = Vec::new();
    buf.push(SOFTNPU_UART_PREAMBLE);
    buf.extend_from_slice(&serde_json::to_vec(&msg).unwrap());
    buf.push(b'\n');

    f.write_all(&buf).unwrap();
    f.sync_all().unwrap();

    let mut buf = [0u8; 10240];
    let mut i = 0;
    loop {
        let n = f.read(&mut buf[i..]).unwrap();
        i += n;
        if buf[i - 1] == b'\n' {
            break;
        }
    }
    let s = String::from_utf8_lossy(&buf[..i - 1]).to_string();

    serde_json::from_str(&s).unwrap()
}
