use devinfo::{get_devices, DiPropValue};
use indicatif::{ProgressBar, ProgressStyle};
use p9ds::proto::{P9Version, Rclunk, Rwrite, Tclunk, Twrite, Version};
use p9kp::Client;
use slog::{Drain, Logger};
use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

async fn get_p9_client() -> Option<p9kp::ChardevClient> {
    let devices = get_devices(false).unwrap();

    // look for libvirt/vritfs device
    let vendor_id = 0x1af4;
    let device_id = 0x1009;

    for (device_key, dev_info) in devices {
        let vendor_match = match dev_info.props.get("vendor-id") {
            Some(value) => value.matches_int(vendor_id),
            _ => false,
        };
        let dev_match = match dev_info.props.get("device-id") {
            Some(value) => value.matches_int(device_id),
            _ => false,
        };
        let unit_address = match dev_info.props.get("unit-address") {
            Some(DiPropValue::Strings(vs)) => {
                if vs.is_empty() {
                    continue;
                }
                vs[0].clone()
            }
            _ => continue,
        };
        if vendor_match && dev_match {
            let dev_path = format!(
                "/devices/pci@0,0/{}@{}:9p",
                device_key.node_name, unit_address,
            );
            let pb = PathBuf::from(dev_path);
            let mut client = p9kp::ChardevClient::new(pb, 0x10000, logger());

            let mut ver = Version::new(P9Version::V2000P4);
            ver.msize = 0x10000;
            let server_version =
                client.send::<Version, Version>(&ver).await.unwrap();
            if Some(P9Version::V2000P4)
                == P9Version::from_str(&server_version.version)
            {
                return Some(client);
            }
        }
    }
    None
}

fn logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_envlogger::new(drain).fuse();
    let drain = slog_async::Async::new(drain).build().fuse();
    Logger::root(drain, slog::o!())
}

pub async fn load_program(
    path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut file = File::open(path)?;
    let mut buf = Vec::new();
    file.read_to_end(&mut buf)?;

    let pb = ProgressBar::new(buf.len() as u64);
    let sty = ProgressStyle::with_template(
        "[{elapsed_precise}] \
                {bar:40.cyan/blue} \
                {bytes}/{total_bytes} \
                {msg}",
    )?
    .progress_chars("##-");

    pb.set_style(sty);

    let mut client = match get_p9_client().await {
        Some(c) => Ok(c),
        None => Err("no p9 device found"),
    }?;

    let mut i = 0;
    let stride = 0x10000 - 23;
    let end = buf.len();
    loop {
        let j = std::cmp::min(i + stride, end);
        let req = Twrite::new(buf[i..j].to_owned(), 0, i as u64);
        let resp: Rwrite = client.send(&req).await?;
        pb.inc(resp.count as u64);
        i += stride;
        if i >= end {
            break;
        }
    }
    pb.finish_with_message("done");
    println!();

    let req = Tclunk::new(0);
    let _resp: Rclunk = client.send(&req).await?;

    Ok(())
}
