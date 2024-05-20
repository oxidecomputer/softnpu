// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

pub mod cli;
pub mod config;
pub mod p9;

use softnpu::p4rs::TableEntry;
use softnpu::ManagementRequest;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::os::unix::io::AsRawFd;

const SOFTNPU_UART_PREAMBLE: u8 = 0b11100101;

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
