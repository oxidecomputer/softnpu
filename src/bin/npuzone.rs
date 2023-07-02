use clap::Parser;
use curl::easy::Easy;
use libnet::{delete_link, LinkFlags, LinkHandle};
use softnpu::cli::get_styles;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use zone::{Adm, Config};
use ztest::{FsMount, SimnetLink, Zfs, Zone};

/// This tool constructs a SoftNPU zone, optionally with an acompanying host
/// zone.
///
///        host         softnpu
///  ┌─────────────┐┌─────────────┐
///  │             ││ ┌ ─ ─ ─ ─ ┐ │       ┌ ─ ─ ─ ─ ┐
///  │┌ ─ ─ ┐┌────┐││┌────┐ ┌────┐│┌────┐
///  │       │shx0├┼┼┤shi0│ │spi0├┼┤spx0├─┤         │
///  ││     │└────┘││└────┘ └────┘│└────┘
///  │  svc        ││ │  ASIC   │ │       │ network │
///  ││     │┌────┐││┌────┐ ┌────┐│┌────┐
///  │       │shx1├┼┼┤shi1│ │spi1├┼┤spx1├─┤         │
///  │└ ─ ─ ┘└────┘││└────┘ └────┘│└────┘
///  │             ││ └ ─ ─ ─ ─ ┘ │       └ ─ ─ ─ ─ ┘
///  │       ┌────┐││┌────┐       │
///  │       │mgtc├┼┼┤mgts│       │
///  │       └────┘││└────┘       │
///  └─────────────┘└─────────────┘
#[derive(Parser, Debug)]
#[command(version, about, long_about = None, styles = get_styles())]
struct Cli {
    #[clap(subcommand)]
    subcmd: SubCommand,
}

#[derive(Parser, Debug)]
enum SubCommand {
    /// Create a SoftNpu zone
    Create(ZoneInfo),

    /// Destroy a SoftNpu zone
    Destroy(ZoneInfo),
}

#[derive(Parser, Debug)]
struct ZoneInfo {
    /// Name of the SoftNpu zone
    name: String,

    /// Number of ports
    #[clap(short, long)]
    ports: usize,

    /// Also create the host zone and plumb in ports.
    #[clap(long)]
    with_host: bool,
}

#[derive(Default)]
struct Resources {
    ports: Vec<SimnetLink>,
    zones: Vec<Zone>,
    zfs: Vec<Zfs>,
}

const SOFTNPU_ZONE_NAME_SUFFIX: &str = "softnpu";
const HOST_ZONE_NAME_SUFFIX: &str = "host";
const ZONE_BRAND: &str = "sparse";
const PFEXEC: &str = "/bin/pfexec";

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.subcmd {
        SubCommand::Create(z) => {
            let mut resources = Resources::default();
            fetch_required_artifacts(&z.name).await?;
            create_ports(&z.name, z.ports, &mut resources)?;
            create_zones(&z.name, z.with_host, &mut resources)?;
            // Exit without calling cleanup destructors. We want the resources
            // to stay! If we exit due to a question mark, things will be
            // cleaned up.
            std::process::exit(0);
        }
        SubCommand::Destroy(z) => {
            destroy_zones(&z.name, z.with_host);
            destroy_ports(&z.name, z.ports);
            std::fs::remove_dir_all(format!("/{}", z.name))?;
        }
    }

    Ok(())
}

async fn fetch_required_artifacts(name: &str) -> anyhow::Result<()> {
    fetch_softnpu(name).await?;
    fetch_sidecar_lite(name).await?;
    fetch_scadm(name).await?;
    Ok(())
}

async fn fetch_head_softnpu_commit() -> anyhow::Result<String> {
    Ok(octocrab::instance()
        .repos("oxidecomputer", "softnpu")
        .list_commits()
        .page(1u32)
        .per_page(1)
        .send()
        .await?
        .take_items()[0]
        .sha
        .clone())
}

async fn fetch_softnpu_url(shasum: bool) -> anyhow::Result<String> {
    let rev = fetch_head_softnpu_commit().await?;
    let base = "https://buildomat.eng.oxide.computer";
    let path = "public/file/oxidecomputer/softnpu/image";
    let file = if shasum {
        "softnpu.sha256.txt"
    } else {
        "softnpu"
    };
    Ok(format!("{}/{}/{}/{}", base, path, rev, file))
}

async fn fetch_sidecar_lite_url(shasum: bool) -> anyhow::Result<String> {
    // hardcode for now until repo is open
    let rev = "8f8e75371d7569ccb2d4112da7b69648050ba9a8";
    let base = "https://buildomat.eng.oxide.computer";
    let path = "public/file/oxidecomputer/sidecar-lite/release";
    let file = if shasum {
        "libsidecar_lite.so.sha256.txt"
    } else {
        "libsidecar_lite.so"
    };
    Ok(format!("{}/{}/{}/{}", base, path, rev, file))
}

async fn fetch_scadm_url(shasum: bool) -> anyhow::Result<String> {
    // hardcode for now until repo is open
    let rev = "8f8e75371d7569ccb2d4112da7b69648050ba9a8";
    let base = "https://buildomat.eng.oxide.computer";
    let path = "public/file/oxidecomputer/sidecar-lite/release";
    let file = if shasum { "scadm.sha256.txt" } else { "scadm" };
    Ok(format!("{}/{}/{}/{}", base, path, rev, file))
}

fn runtime_dir(name: &str) -> String {
    format!("/var/run/softnpu/{}", name)
}

async fn fetch_sidecar_lite(name: &str) -> anyhow::Result<()> {
    fetch_artifact(
        &fetch_sidecar_lite_url(false).await?,
        &fetch_sidecar_lite_url(true).await?,
        "asic_program.so",
        name,
    )
    .await
}

async fn fetch_scadm(name: &str) -> anyhow::Result<()> {
    fetch_artifact(
        &fetch_scadm_url(false).await?,
        &fetch_scadm_url(true).await?,
        "scadm",
        name,
    )
    .await
}

async fn fetch_softnpu(name: &str) -> anyhow::Result<()> {
    fetch_artifact(
        &fetch_softnpu_url(false).await?,
        &fetch_softnpu_url(true).await?,
        "softnpu",
        name,
    )
    .await
}

async fn fetch_artifact(
    artifact_url: &str,
    shasum_url: &str,
    filename: &str,
    name: &str,
) -> anyhow::Result<()> {
    let mut easy = Easy::new();
    let name = name.to_owned();

    let mut remote_shasum = Vec::new();
    easy.url(shasum_url).unwrap();
    let mut transfer = easy.transfer();
    transfer.write_function(|data| {
        remote_shasum.extend_from_slice(data);
        Ok(data.len())
    })?;
    transfer.perform()?;
    drop(transfer);

    let remote_shasum =
        String::from_utf8_lossy(remote_shasum.as_slice()).to_string();
    let remote_shasum = remote_shasum.trim().to_owned();

    let dir = runtime_dir(&name);
    std::fs::create_dir_all(&dir)?;
    let filepath = format!("{}/{}", dir, filename);
    let path = std::path::Path::new(&filepath);

    if let Ok(digest) = sha256::try_digest(path) {
        if digest == remote_shasum {
            println!("already have latest {filename}");
            return Ok(());
        } else {
            println!(
                "{filename} shasum mismatch \n{:?} != {:?}",
                digest, remote_shasum,
            );
            println!("will update {filename}")
        }
    }

    print!("downloading {filename} ...");
    std::io::stdout().flush()?;

    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .append(true)
        .mode(0o755)
        .open(path)
        .unwrap();

    easy.url(artifact_url).unwrap();
    let mut transfer = easy.transfer();
    transfer.write_function(|data| {
        f.write_all(data).unwrap();
        Ok(data.len())
    })?;
    transfer.perform()?;
    drop(transfer);

    println!("done");

    Ok(())
}

fn create_ports(
    name: &str,
    count: usize,
    resources: &mut Resources,
) -> anyhow::Result<()> {
    let mut cfg = softnpu::config::Config::default();
    for i in 0..count {
        println!("creating port {}", i);
        let external_ifx = format!("{}_shx{}", name, i);
        let internal_ifx = format!("{}_shi{}", name, i);
        let scrimlet = internal_ifx.clone();
        resources
            .ports
            .push(SimnetLink::new(&external_ifx, &internal_ifx)?);

        let external_ifx = format!("{}_spx{}", name, i);
        let internal_ifx = format!("{}_spi{}", name, i);
        let sidecar = internal_ifx.clone();
        resources
            .ports
            .push(SimnetLink::new(&external_ifx, &internal_ifx)?);
        cfg.ports.push(softnpu::config::Port {
            sidecar,
            scrimlet,
            mtu: 1500,
        });

        cfg.p4_program = "/softnpu/asic_program.so".to_owned();
    }
    let filepath = format!("{}/softnpu.toml", runtime_dir(name));
    let mut f = std::fs::File::create(filepath)?;
    f.write_all(toml::to_string(&cfg)?.as_bytes())?;
    Ok(())
}

fn destroy_ports(name: &str, count: usize) {
    for i in 0..count {
        destroy_port(format!("{}_shx{}", name, i));
        destroy_port(format!("{}_shi{}", name, i));
        destroy_port(format!("{}_spx{}", name, i));
        destroy_port(format!("{}_spi{}", name, i));
    }
}

fn destroy_port(name: String) {
    println!("destroying port {}", name);
    let h = LinkHandle::Name(name.clone());
    if let Err(e) = delete_link(&h, LinkFlags::Active) {
        eprintln!("failed to delete link {}: {}", name, e);
    }
}

fn create_zones(
    name: &str,
    with_host: bool,
    resources: &mut Resources,
) -> anyhow::Result<()> {
    let zfs = Zfs::new(name)?;
    println!("softnpu zone setup");
    create_softnpu_zone(name, resources, &zfs)?;
    if with_host {
        println!("host zone setup");
        create_host_zone(name, resources, &zfs)?;
    }
    resources.zfs.push(zfs);
    copy_in_mgmt_script(name)?;
    Ok(())
}

fn destroy_zones(name: &str, with_host: bool) {
    if let Err(e) = std::process::Command::new(PFEXEC)
        .env_clear()
        .arg("zfs")
        .arg("destroy")
        .arg("-rf")
        .arg(&format!("rpool/{}", name))
        .output()
    {
        eprintln!("failed to delete zfs dataset rpool/{name}: {e}")
    }

    destroy_zone(format!("{}_{}", name, SOFTNPU_ZONE_NAME_SUFFIX));
    if with_host {
        destroy_zone(format!("{}_{}", name, HOST_ZONE_NAME_SUFFIX));
    }
}

fn destroy_zone(name: String) {
    println!("halting zone {name}");
    if let Err(e) = Adm::new(&name).halt_blocking() {
        eprintln!("failed to halt zone config for {name}: {e}");
    }

    println!("uninstalling zone {name}");
    if let Err(e) = Adm::new(&name).uninstall_blocking(true) {
        eprintln!("failed to uninstall zone config for {name}: {e}");
    }

    println!("deleting zone {name}");
    let mut c = Config::new(&name);
    if let Err(e) = c.delete(true).run_blocking() {
        eprintln!("failed to delete zone config for {name}: {e}");
    }
}

fn create_softnpu_zone(
    name: &str,
    resources: &mut Resources,
    zfs: &Zfs,
) -> anyhow::Result<()> {
    let phys: Vec<&str> =
        resources.ports.iter().map(|p| p.end_b.as_str()).collect();

    let dir = runtime_dir(name);
    let fs = FsMount::new(&dir, "/softnpu");

    resources.zones.push(Zone::new(
        &format!("{}_{}", name, SOFTNPU_ZONE_NAME_SUFFIX),
        ZONE_BRAND,
        zfs,
        &phys,
        &[fs],
    )?);
    Ok(())
}

fn create_host_zone(
    name: &str,
    resources: &mut Resources,
    zfs: &Zfs,
) -> anyhow::Result<()> {
    // host ports are only the first half
    let n = resources.ports.len() / 2;
    let phys: Vec<&str> = resources.ports[..n]
        .iter()
        .map(|p| p.end_a.as_str())
        .collect();

    let dir = runtime_dir(name);
    let fs = FsMount::new(&dir, "/softnpu");

    resources.zones.push(Zone::new(
        &format!("{}_{}", name, HOST_ZONE_NAME_SUFFIX),
        ZONE_BRAND,
        zfs,
        &phys,
        &[fs],
    )?);
    Ok(())
}

fn copy_in_mgmt_script(name: &str) -> anyhow::Result<()> {
    let path = format!("{}/npu", runtime_dir(name));
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .mode(0o755)
        .open(path)
        .unwrap();
    f.write_all(SOFTNPU_SCRIPT.as_bytes())?;
    Ok(())
}

const SOFTNPU_SCRIPT: &str = "#!/bin/bash

case $1 in
    start) 
        /softnpu/softnpu --uds-path /softnpu /softnpu/softnpu.toml &
        ;;
    stop)
        pkill -9 softnpu
        ;;
    *)
        echo 'usage: npu [start|stop]'
        exit 1
        ;;
esac
";
