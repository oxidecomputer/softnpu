// This Source Code Form is subject to the terms of the Mozilla Public
// License, v. 2.0. If a copy of the MPL was not distributed with this
// file, You can obtain one at https://mozilla.org/MPL/2.0/.

pub mod cli;
pub mod config;
#[cfg(feature = "p9")]
pub mod p9;

use softnpu::p4rs::TableEntry;
use softnpu::ManagementRequest;
use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
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
        // On illumos, VMIN and VTIME share c_cc slots with VEOF and VEOL.
        // After clearing ICANON, they hold whatever the canonical values
        // were set beforehand.
        //
        // These values come from ldterm, which is torn down on the tty's last
        // close and rebuilt from the root "ttymodes" property on next open; a
        // fresh open starts canonically with VEOF=4 and VEOL=0, and nothing
        // set here outlives this process unless another holder keeps the
        // stream alive. In non-canonical mode, read(2) follows the MIN/TIME
        // cases:
        //   - MIN > 0, TIME = 0: block until MIN bytes are queued
        //   - MIN > 0, TIME > 0: wait for MIN bytes or an inter-byte timeout
        //   - MIN = 0, TIME > 0: overall timer, may return 0 bytes
        //   - MIN = 0, TIME = 0: poll, returns 0 bytes on an empty queue
        //
        // Propolis (hw/virtio/softnpu.rs, DumpRequest) serializes the whole
        // table map as one JSON line and writes it into the emulated 16550
        // UART behind this tty one byte at a time, yielding whenever its
        // one-byte FIFO is full. The guest can then see it in short bursts with
        // gaps between them, where a large dump of contents could mean
        // thousands of these bursts.
        //
        // read_until treats a zero-length read as EOF. Both MIN > 0 cases
        // avoid zero-length timeout reads. With TIME = 0, MIN must be 1
        // because the trailing newline may arrive on its own.
        //
        // See termio(4I) and POSIX termios.h:
        // <https://illumos.org/man/4I/termio>
        // <https://pubs.opengroup.org/onlinepubs/9699919799/basedefs/termios.h.html>
        term.c_cc[libc::VMIN] = 1;
        term.c_cc[libc::VTIME] = 0;
        // TCSAFLUSH discards input already queued. An interrupted dump may
        // still be transmitting; bytes arriving after the flush remain.
        if libc::tcsetattr(fd, libc::TCSAFLUSH, &term) != 0 {
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

    let mut buf = Vec::new();
    BufReader::new(&mut f).read_until(b'\n', &mut buf).unwrap();
    let s = String::from_utf8_lossy(buf.strip_suffix(b"\n").unwrap_or(&buf));

    serde_json::from_str(&s).unwrap()
}
