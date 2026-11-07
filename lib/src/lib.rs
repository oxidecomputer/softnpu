// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

use p4rs::{Pipeline, TableEntry};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
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
    TableClear(TableClear),
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

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct TableClear {
    pub table: String,
}

pub async fn handle_management_message(
    msg: ManagementRequest,
    pipeline: &Mutex<Box<dyn Pipeline>>,
    uds: &UnixDatagram,
    uds_dst: &Path,
    radix: usize,
) -> io::Result<()> {
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
        ManagementRequest::TableClear(tc) => {
            let mut pl = pipeline.lock().unwrap();
            if !pl.clear_table_entries(&tc.table) {
                return Err(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("unknown table: {}", tc.table),
                ));
            }
        }
        ManagementRequest::RadixRequest => {
            let response = ManagementResponse::RadixResponse(radix as u16);
            send_response(response, uds, uds_dst).await?;
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
            send_response(response, uds, uds_dst).await?;
        }
    }
    Ok(())
}

async fn send_response(
    response: ManagementResponse,
    uds: &UnixDatagram,
    uds_dst: &Path,
) -> io::Result<()> {
    let buf = serde_json::to_vec(&response).map_err(io::Error::other)?;
    uds.send_to(&buf, uds_dst).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPipeline {
        tables: BTreeMap<String, Vec<TableEntry>>,
    }

    impl Default for TestPipeline {
        fn default() -> Self {
            let allow_source = |keyset_data: Vec<u8>| TableEntry {
                action_id: "allow_source".to_string(),
                keyset_data,
                parameter_data: Vec::new(),
            };
            // Missing ids in this map are considered unknown tables.
            let tables = BTreeMap::from([
                // IPv4 source filter key: 192.0.2.1/32, dst 224.1.2.3 (LE).
                (
                    "populated".to_string(),
                    vec![allow_source(vec![192, 0, 2, 1, 32, 3, 2, 1, 224])],
                ),
                ("empty".to_string(), Vec::new()),
                (
                    "other".to_string(),
                    vec![allow_source(vec![192, 0, 2, 2, 32, 4, 2, 1, 224])],
                ),
            ]);
            Self { tables }
        }
    }

    impl Pipeline for TestPipeline {
        fn process_packet<'a>(
            &mut self,
            _port: u16,
            _packet: &mut p4rs::packet_in<'a>,
        ) -> Vec<(p4rs::packet_out<'a>, u16)> {
            unreachable!()
        }

        fn add_table_entry(
            &mut self,
            _table: &str,
            _action: &str,
            _keys: &[u8],
            _parameters: &[u8],
            _priority: u32,
        ) {
            unreachable!()
        }

        fn remove_table_entry(&mut self, _table: &str, _keys: &[u8]) {
            unreachable!()
        }

        fn clear_table_entries(&mut self, table: &str) -> bool {
            match self.tables.get_mut(table) {
                Some(entries) => {
                    entries.clear();
                    true
                }
                None => false,
            }
        }

        fn get_table_entries(&self, table: &str) -> Option<Vec<TableEntry>> {
            self.tables.get(table).map(|entries| {
                entries
                    .iter()
                    .map(|entry| TableEntry {
                        action_id: entry.action_id.clone(),
                        keyset_data: entry.keyset_data.clone(),
                        parameter_data: entry.parameter_data.clone(),
                    })
                    .collect()
            })
        }

        fn get_table_ids(&self) -> Vec<&str> {
            unreachable!()
        }
    }

    #[tokio::test]
    async fn table_clear_empties_only_requested_table() {
        let dir = tempfile::tempdir().unwrap();
        let server = UnixDatagram::bind(dir.path().join("server")).unwrap();
        let client_path = dir.path().join("client");
        let pipeline: Mutex<Box<dyn Pipeline>> =
            Mutex::new(Box::new(TestPipeline::default()));
        let table_lens = |pipeline: &Mutex<Box<dyn Pipeline>>| {
            let pl = pipeline.lock().unwrap();
            ["populated", "empty", "other", "missing"]
                .map(|table| pl.get_table_entries(table).as_ref().map(Vec::len))
        };
        assert_eq!(table_lens(&pipeline), [Some(1), Some(0), Some(1), None]);

        for (table, expected) in [
            ("populated", [Some(0), Some(0), Some(1), None]),
            ("populated", [Some(0), Some(0), Some(1), None]),
            ("empty", [Some(0), Some(0), Some(1), None]),
            ("missing", [Some(0), Some(0), Some(1), None]),
            ("other", [Some(0), Some(0), Some(0), None]),
        ] {
            let request = ManagementRequest::TableClear(TableClear {
                table: table.to_string(),
            });
            let result = handle_management_message(
                request,
                &pipeline,
                &server,
                &client_path,
                4,
            )
            .await;
            if table == "missing" {
                assert_eq!(result.unwrap_err().kind(), io::ErrorKind::NotFound);
            } else {
                result.unwrap();
            }
            assert_eq!(
                table_lens(&pipeline),
                expected,
                "after clearing {table}"
            );
        }
    }
}
