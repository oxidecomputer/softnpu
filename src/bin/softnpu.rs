// Copyright 2022 Oxide Computer Company

use anyhow::{anyhow, Result};
use clap::Parser;
use dlpi::{sys::dlpi_recvinfo_t, DlpiHandle};
use libloading::os::unix::{Library, Symbol, RTLD_NOW};
use p4rs::{packet_in, packet_out, Pipeline};
use slog::{error, info, warn, Drain, Logger};
use softnpu::cli::get_styles;
use softnpu::config::{Config, Port};
use softnpu::mgmt;
use std::fs::read_to_string;
use std::sync::Arc;
use tokio::net::UnixDatagram;
use tokio::sync::Mutex;

const L2_HEADROOM: usize = 20;

#[derive(Clone)]
pub struct Switch {
    pub ports: Vec<SwitchPort>,
}

#[derive(Clone, Copy)]
pub struct SwitchPort {
    pub sidecar: DlpiHandle,
    pub scrimlet: DlpiHandle,
    pub mtu: usize,
}

impl Switch {
    fn new(ports: &Vec<Port>) -> Result<Switch> {
        Ok(Switch {
            ports: Self::init_ports(ports)?,
        })
    }

    fn init_ports(ports: &Vec<Port>) -> Result<Vec<SwitchPort>> {
        let mut result = Vec::new();
        for p in ports {
            let sidecar =
                Self::init_port(&p.sidecar).map_err(|e| {
                    anyhow!(
                        "initializing sidecar port {} failed: {}",
                        p.sidecar,
                        e,
                    )
                })?;
            let scrimlet = Self::init_port(&p.scrimlet).map_err(|e| {
                anyhow!(
                    "initializing scrimlet port {} failed: {}",
                    p.sidecar,
                    e,
                )
            })?;
            result.push(SwitchPort {
                sidecar,
                scrimlet,
                mtu: p.mtu,
            })
        }
        Ok(result)
    }

    fn init_port(devname: &str) -> Result<DlpiHandle> {
        let p = dlpi::open(devname, dlpi::sys::DLPI_RAW)?;
        dlpi::bind(p, 0x86dd)?;
        dlpi::promisc_on(p, dlpi::sys::DL_PROMISC_MULTI)?;
        dlpi::promisc_on(p, dlpi::sys::DL_PROMISC_SAP)?;
        dlpi::promisc_on(p, dlpi::sys::DL_PROMISC_PHYS)?;
        dlpi::promisc_on(p, dlpi::sys::DL_PROMISC_RX_ONLY)?;
        Ok(p)
    }
}

#[derive(Parser, Debug)]
#[command(version, about, long_about = None, styles = get_styles())]
struct Cli {
    /// soft-npu configuration file path
    config: String,

    /// UDS socket path
    #[arg(long, default_value = "/stuff")]
    uds_path: String,
}

fn load_program(path: &str) -> Result<(Library, Box<dyn Pipeline>)> {
    let lib = match unsafe { Library::open(Some(&path), RTLD_NOW) } {
        Ok(l) => l,
        Err(e) => return Err(anyhow!("failed to load p4 program: {}", e)),
    };
    let func: Symbol<unsafe extern "C" fn() -> *mut dyn Pipeline> =
        match unsafe { lib.get(b"_main_pipeline_create") } {
            Ok(f) => f,
            Err(e) => {
                return Err(anyhow!(
                    "failed to load _main_pipeline_create func: {}",
                    e
                ))
            }
        };

    let boxpipe = unsafe { Box::from_raw(func()) };
    Ok((lib, boxpipe))
}

fn init_logger() -> Logger {
    let decorator = slog_term::TermDecorator::new().build();
    let drain = slog_term::FullFormat::new(decorator).build().fuse();
    let drain = slog_envlogger::new(drain).fuse();
    let drain = slog_async::Async::new(drain)
        .chan_size(0x2000)
        .build()
        .fuse();
    slog::Logger::root(drain, slog::o!())
}

async fn run_ingress_packet_handler(
    index: usize,
    switch: Switch,
    pipeline: Arc<Mutex<Box<dyn Pipeline>>>,
    log: Logger,
) {
    info!(log, "ingress packet handler is running for port {}", index);
    let dh = switch.ports[index].sidecar;
    let mtu = switch.ports[index].mtu + L2_HEADROOM;
    loop {
        let mut src = [0u8; dlpi::sys::DLPI_PHYSADDR_MAX];
        let mut msg = vec![0u8; mtu];
        let mut recvinfo = dlpi_recvinfo_t::default();
        let n =
            match dlpi::recv_async(dh, &mut src, &mut msg, Some(&mut recvinfo))
                .await
            {
                Ok((_, n)) => n,
                Err(e) => {
                    error!(log, "rx error at index {}: {}", index, e);
                    continue;
                }
            };

        // TODO pipeline should not need to be mutable for packet handling?
        let pkt = packet_in::new(&msg[..n]);
        let mut pl = pipeline.lock().await;

        handle_external_packet(index + 1, pkt, &switch, &mut pl, &log).await;
    }
}

async fn run_egress_packet_handler(
    index: usize,
    switch: Switch,
    pipeline: Arc<Mutex<Box<dyn Pipeline>>>,
    log: Logger,
) {
    info!(log, "egress packet handler is running for port {}", index);
    let dh = switch.ports[index].scrimlet;
    let mtu = switch.ports[index].mtu + L2_HEADROOM;
    loop {
        let mut src = [0u8; dlpi::sys::DLPI_PHYSADDR_MAX];
        let mut msg = vec![0u8; mtu];
        let mut recvinfo = dlpi_recvinfo_t::default();
        let n =
            match dlpi::recv_async(dh, &mut src, &mut msg, Some(&mut recvinfo))
                .await
            {
                Ok((_, n)) => n,
                Err(e) => {
                    error!(log, "rx error at index {}: {}", index, e);
                    continue;
                }
            };
        let mut frame = vec![0u8; mtu];

        frame[..14].clone_from_slice(&msg[..14]);
        let orig_ethertype = [frame[12], frame[13]];
        frame[12] = 0x09;
        frame[13] = 0x01;
        // set up sidecar header
        let portnum = (index as u16 + 1).to_be_bytes();
        frame[14] = 0; // sc_code = forward from userspace
        frame[15] = 0; // pad
        frame[16] = portnum[0];
        frame[17] = portnum[1];
        frame[18] = portnum[0];
        frame[19] = portnum[1];
        frame[20] = orig_ethertype[0];
        frame[21] = orig_ethertype[1];
        let m = n + 24;
        // 38 = 14 (eth) + 24 (sidecar)
        frame[38..m].clone_from_slice(&msg[14..n]);

        // TODO pipeline should not need to be mutable for packet handling?
        let pkt = packet_in::new(&frame[..m]);
        let mut pl = pipeline.lock().await;

        handle_internal_packet(index + 1, pkt, &switch, &mut pl, &log).await
    }
}

async fn handle_internal_packet<'a>(
    index: usize,
    mut pkt: packet_in<'a>,
    switch: &Switch,
    pipeline: &mut Box<dyn Pipeline>,
    log: &Logger,
) {
    for (mut out_pkt, port) in pipeline.process_packet(index as u16, &mut pkt) {
        if port == 0 {
            warn!(
                log,
                "egress port=0 for internal packet on index {}, dropping",
                index,
            );
            continue;
        }
        handle_packet_to_ext_port(&mut out_pkt, switch, port, log);
    }
}

async fn handle_external_packet<'a>(
    index: usize,
    mut pkt: packet_in<'a>,
    switch: &Switch,
    pipeline: &mut Box<dyn Pipeline>,
    log: &Logger,
) {
    for (mut out_pkt, port) in pipeline.process_packet(index as u16, &mut pkt) {
        // packet is going to CPU port
        if port == 0 {
            handle_packet_to_cpu_port(&mut out_pkt, switch, log).await;
        }
        // packet is passing through
        else {
            handle_packet_to_ext_port(&mut out_pkt, switch, port, log);
        }
    }
}

fn handle_packet_to_ext_port(
    pkt: &mut packet_out<'_>,
    switch: &Switch,
    port: u16,
    log: &Logger,
) {
    let dh = switch.ports[port as usize - 1].sidecar;

    //TODO avoid copying the whole packet
    let mut out = pkt.header_data.clone();
    out.extend_from_slice(pkt.payload_data);

    match dlpi::send(dh, &[], out.as_slice(), None) {
        Ok(_) => {}
        Err(e) => {
            error!(log, "tx (ext,0): {}", e);
        }
    }
}

async fn handle_packet_to_cpu_port<'a>(
    pkt: &mut packet_out<'a>,
    switch: &Switch,
    log: &Logger,
) {
    // get the destination port
    // 16 =
    //   size_of(ethernet) = 14 +
    //   offset(sidecar.sc_egress) = 2
    let portnum =
        u16::from_be_bytes([pkt.header_data[16], pkt.header_data[17]]) as usize;

    if portnum == 0 {
        warn!(log, "got sidecar egress port of 0");
        warn!(log, "this is probably a p4 program bug");
        warn!(log, "dropping packet");
        return;
    }

    if portnum > switch.ports.len() {
        warn!(
            log,
            "portnum too large, {} of {}",
            portnum - 1,
            switch.ports.len() - 1
        );
        return;
    }
    let dh = switch.ports[portnum - 1].scrimlet;

    // replace sidecar ethertype with encapsulated packet ethertype
    let et0 = pkt.header_data[20];
    let et1 = pkt.header_data[21];
    let eth = &mut pkt.header_data.as_mut_slice()[..14];
    eth[12] = et0;
    eth[13] = et1;

    //TODO avoid copying the whole packet
    let mut out = eth.to_vec();
    // skip sidecar header and write out L3 header
    // 38 = 14 (eth) + 24 (sidecar)
    let l3 = &pkt.header_data.as_mut_slice()[38..];
    out.extend_from_slice(l3);
    out.extend_from_slice(pkt.payload_data);

    match dlpi::send(dh, &[], out.as_slice(), None) {
        Ok(_) => {}
        Err(e) => {
            error!(log, "tx (int,0): {}", e);
        }
    }
}

#[tokio::main]
async fn main() {
    let log = init_logger();

    if let Err(e) = run(log.clone()).await {
        error!(log, "{}", e);
    }
}

async fn run(log: Logger) -> Result<()> {
    let cli = Cli::parse();
    let txt = read_to_string(&cli.config)
        .map_err(|e| anyhow!("read config file {} error: {}", cli.config, e))?;

    let config: Config = toml::from_str(&txt).map_err(|e| {
        anyhow!("parse config file {} error: {}", cli.config, e)
    })?;

    println!("{:#?}", config);

    let sw = Switch::new(&config.ports)?;
    let (_lib, pipe) = load_program(&config.p4_program)?;
    let pipe = Arc::new(Mutex::new(pipe));

    for (index, _) in sw.ports.iter().enumerate() {
        let sw_ = sw.clone();
        let pipe_ = pipe.clone();
        let log_ = log.clone();
        tokio::spawn(async move {
            run_ingress_packet_handler(index, sw_, pipe_, log_).await;
        });
        let sw_ = sw.clone();
        let pipe_ = pipe.clone();
        let log_ = log.clone();
        tokio::spawn(async move {
            run_egress_packet_handler(index, sw_, pipe_, log_).await;
        });
    }

    //TODO as parameters
    let server = format!("{}/server", cli.uds_path);
    let client = format!("{}/client", cli.uds_path);

    let _ = std::fs::remove_file(&server);

    let uds = Arc::new(
        UnixDatagram::bind(&server)
            .map_err(|e| anyhow!("failed to open management socket: {}", e))?,
    );

    loop {
        let mut buf = vec![0u8; 10240];
        let n = match uds.recv(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                error!(log, "management socket recv: {}", e);
                continue;
            }
        };
        let msg: mgmt::ManagementRequest =
            match serde_json::from_slice(&buf[..n]) {
                Ok(msg) => msg,
                Err(_) => continue,
            };

        mgmt::handle_management_message(
            msg,
            pipe.clone(),
            uds.clone(),
            &client,
            config.ports.len(),
            log.clone(),
        )
        .await;
    }
}
