use camino::Utf8PathBuf;
use clap::Parser;
use curl::easy::Easy;
use flate2::read::GzDecoder;
use indicatif::{ProgressBar, ProgressStyle};
use smf::{scf_type_t, Scf, ScfError};
use softnpu_client::cli::get_styles;
use std::fs;
use std::io::{self, Write};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use tar::Archive;
use tempfile::TempDir;

#[derive(Parser, Debug)]
#[command(version, about, long_about = None, styles = get_styles())]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Parser, Debug)]
enum Command {
    /// Install NPU VM machinery
    Install(Install),

    /// Destroy NPU VM machinery
    Uninstall,
}

/// Artifact version determines how aritfacts are fetched.
pub enum ArtifactVersion<'a> {
    Latest,
    Branch(&'a str),
    Commit(&'a str),
}

#[derive(Parser, Debug)]
struct Install {
    /// When branch is specified, this tool will look up the most recent commit
    /// on the specified branch and fetch artifacts for that commit.
    #[clap(long)]
    dendrite_branch: Option<String>,

    /// When commit is specified, this tool will fetch artifacts for the
    /// specified commit.
    #[clap(long, conflicts_with = "dendrite-branch")]
    dendrite_commit: Option<String>,

    /// When branch is specified, this tool will look up the most recent commit
    /// on the specified branch and fetch artifacts for that commit.
    #[clap(long)]
    sidecar_lite_branch: Option<String>,

    /// When commit is specified, this tool will fetch artifacts for the
    /// specified commit.
    #[clap(long, conflicts_with = "sidecar-lite-branch")]
    sidecar_lite_commit: Option<String>,

    /// The number of front-facing switch ports
    #[clap(long)]
    front_ports: usize,

    /// The number of rear-facing switch ports
    #[clap(long)]
    rear_ports: usize,

    /// The packet source device for tfport (e.g., "vioif0")
    #[clap(long)]
    pkt_source: String,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::Install(cmd) => {
            let dendrite = if let Some(branch) = &cmd.dendrite_branch {
                ArtifactVersion::Branch(branch.as_str())
            } else if let Some(commit) = &cmd.dendrite_commit {
                ArtifactVersion::Commit(commit.as_str())
            } else {
                ArtifactVersion::Latest
            };

            let sidecar_lite = if let Some(branch) = &cmd.sidecar_lite_branch {
                ArtifactVersion::Branch(branch.as_str())
            } else if let Some(commit) = &cmd.sidecar_lite_commit {
                ArtifactVersion::Commit(commit.as_str())
            } else {
                ArtifactVersion::Latest
            };

            install(
                dendrite,
                sidecar_lite,
                cmd.front_ports,
                cmd.rear_ports,
                &cmd.pkt_source,
            )
            .await?;
        }
        Command::Uninstall => uninstall()?,
    }
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("install dendrite error: {0}")]
    Dendrite(#[from] InstallDendriteError),

    #[error("fetch softnpu artifacts error: {0}")]
    FetchSoftnpuArtifacts(#[from] FetchArtifactsError),
}

pub async fn install<'a>(
    dendrite: ArtifactVersion<'a>,
    sidecar_lite: ArtifactVersion<'a>,
    front_ports: usize,
    rear_ports: usize,
    pkt_source: &str,
) -> Result<(), InstallError> {
    install_dendrite(dendrite, front_ports, rear_ports, pkt_source).await?;
    fetch_sidecar_lite_artifacts(sidecar_lite).await?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum UninstallError {
    #[error("dendrite uninstall error: {0}")]
    Dendrite(#[from] DendriteUninstallError),
}

pub fn uninstall() -> Result<(), UninstallError> {
    uninstall_dendrite()?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DendriteUninstallError {
    #[error("disable dendrite servies error: {0}")]
    DisableServices(#[from] DisableDendriteServicesError),

    #[error("remove dendrite image error: {0}")]
    RemoveImage(#[from] RemoveDendriteImageError),
}

/// Uninstalls dendrite by disabling all services and removing root filesystem
/// image.
pub fn uninstall_dendrite() -> Result<(), DendriteUninstallError> {
    disable_dendrite_services()?;
    remove_dendrite_image()?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum RemoveDendriteImageError {
    #[error("failed to remove /opt/oxide/dendrite: {0}")]
    RemoveDendriteDir(io::Error),

    #[error("failed to remove /var/svc/manifest/site/dendrite: {0}")]
    RemoveDendriteManifest(io::Error),

    #[error("failed to remove /var/svc/manifest/site/tfport: {0}")]
    RemoveTfportManifest(io::Error),

    #[error("failed to remove /var/svc/manifest/site/uplink: {0}")]
    RemoveUplinkManifest(io::Error),

    #[error("failed to remove /usr/bin/swadm symlink: {0}")]
    RemoveSwadmSymlink(io::Error),
}

/// Removes dendrite image by recursive deletion of
/// - /opt/oxide/dendrite
/// - /var/svc/manifest/site/dendrite
/// - /var/svc/manifest/site/tfport
/// - /var/svc/manifest/site/uplink
/// - /usr/bin/swadm (symlink)
pub fn remove_dendrite_image() -> Result<(), RemoveDendriteImageError> {
    use RemoveDendriteImageError as E;

    // Remove /opt/oxide/dendrite if it exists
    let dendrite_dir = "/opt/oxide/dendrite";
    if fs::metadata(dendrite_dir).is_ok() {
        fs::remove_dir_all(dendrite_dir).map_err(E::RemoveDendriteDir)?;
    }

    // Remove /var/svc/manifest/site/dendrite directory if it exists
    let dendrite_manifest = "/var/svc/manifest/site/dendrite";
    if fs::metadata(dendrite_manifest).is_ok() {
        fs::remove_dir_all(dendrite_manifest)
            .map_err(E::RemoveDendriteManifest)?;
    }

    // Remove /var/svc/manifest/site/tfport directory if it exists
    let tfport_manifest = "/var/svc/manifest/site/tfport";
    if fs::metadata(tfport_manifest).is_ok() {
        fs::remove_dir_all(tfport_manifest).map_err(E::RemoveTfportManifest)?;
    }

    // Remove /var/svc/manifest/site/uplink directory if it exists
    let uplink_manifest = "/var/svc/manifest/site/uplink";
    if fs::metadata(uplink_manifest).is_ok() {
        fs::remove_dir_all(uplink_manifest).map_err(E::RemoveUplinkManifest)?;
    }

    // Remove /usr/bin/swadm symlink if it exists
    let swadm_link = "/usr/bin/swadm";
    if fs::symlink_metadata(swadm_link).is_ok() {
        fs::remove_file(swadm_link).map_err(E::RemoveSwadmSymlink)?;
    }

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum InstallDendriteError {
    #[error("fetch dendrite image error: {0}")]
    Fetch(#[from] FetchImageError),

    #[error("failed to copy root filesystem: {0}")]
    CopyRootFilesystem(io::Error),

    #[error("failed to import SMF manifest {path}: {error}")]
    ImportManifest { path: &'static str, error: io::Error },

    #[error("failed to remove dependency {dep} from {service}: {error}")]
    RemoveDependency {
        service: &'static str,
        dep: &'static str,
        error: ScfError,
    },

    #[error("failed to install tofino driver: {0}")]
    InstallTofinoDriver(io::Error),

    #[error("failed to create swadm symlink: {0}")]
    CreateSwadmSymlink(io::Error),

    #[error("enable dendrite services error: {0}")]
    Enable(#[from] EnableDendriteServicesError),
}

/// Install dendrite. This function fetches the softnpu image for the specified
/// dendrite version. Dendrite images come in the form of a root filesystem.
/// This function works by copying the contets of the package to the system
/// root. Then the dendrite, tfport and uplink services are brought onine.
pub async fn install_dendrite<'a>(
    version: ArtifactVersion<'a>,
    front_ports: usize,
    rear_ports: usize,
    pkt_source: &str,
) -> Result<(), InstallDendriteError> {
    use InstallDendriteError as E;

    let extracted_path = fetch_dendrite_image(version).await?;

    // Copy the root filesystem contents to the system root
    // The extracted_path points to a "root" directory containing the filesystem
    let options = fs_extra::dir::CopyOptions {
        overwrite: true,
        skip_exist: false,
        buffer_size: 64000,
        copy_inside: true,
        content_only: true,
        depth: 0,
    };

    fs_extra::dir::copy(&extracted_path, "/", &options)
        .map_err(|e| E::CopyRootFilesystem(io::Error::other(e)))?;

    // Create symlink for swadm
    let swadm_link = std::path::Path::new("/usr/bin/swadm");
    if swadm_link.exists() {
        fs::remove_file(swadm_link).map_err(E::CreateSwadmSymlink)?;
    }
    std::os::unix::fs::symlink("/opt/oxide/dendrite/bin/swadm", swadm_link)
        .map_err(E::CreateSwadmSymlink)?;

    // Import SMF manifests so the services are known to SMF
    for manifest in [
        "/var/svc/manifest/site/dendrite/manifest.xml",
        "/var/svc/manifest/site/tfport/manifest.xml",
        "/var/svc/manifest/site/uplink/manifest.xml",
    ] {
        let status = std::process::Command::new("svccfg")
            .arg("import")
            .arg(manifest)
            .status()
            .map_err(|e| E::ImportManifest {
                path: manifest,
                error: e,
            })?;
        if !status.success() {
            return Err(E::ImportManifest {
                path: manifest,
                error: io::Error::other(format!(
                    "svccfg import exited with status {}",
                    status
                )),
            });
        }
    }

    // Remove dependencies that don't exist in this environment
    let scf = Scf::new().map_err(|e| E::RemoveDependency {
        service: "oxide/dendrite",
        dep: "zone_network_setup",
        error: e,
    })?;
    let scope = scf.scope_local().map_err(|e| E::RemoveDependency {
        service: "oxide/dendrite",
        dep: "zone_network_setup",
        error: e,
    })?;

    for (service, dep) in [
        ("oxide/dendrite", "zone_network_setup"),
        ("oxide/tfport", "zone_network_setup"),
        ("oxide/tfport", "switch_zone_setup"),
    ] {
        let svc = scope
            .get_service(service)
            .map_err(|e| E::RemoveDependency {
                service,
                dep,
                error: e,
            })?
            .ok_or_else(|| E::RemoveDependency {
                service,
                dep,
                error: ScfError::NotFound,
            })?;

        // Find and delete the dependency property group
        for pg in svc.pgs().map_err(|e| E::RemoveDependency {
            service,
            dep,
            error: e,
        })? {
            let pg = pg.map_err(|e| E::RemoveDependency {
                service,
                dep,
                error: e,
            })?;
            let pg_name = pg.name().map_err(|e| E::RemoveDependency {
                service,
                dep,
                error: e,
            })?;
            if pg_name == dep {
                pg.delete().map_err(|e| E::RemoveDependency {
                    service,
                    dep,
                    error: e,
                })?;
                break;
            }
        }
    }

    // Install tofino driver
    let status = std::process::Command::new("pkg")
        .arg("install")
        .arg("tofino")
        .status()
        .map_err(E::InstallTofinoDriver)?;
    // Exit code 4 means the package is already installed
    if !status.success() && status.code() != Some(4) {
        return Err(E::InstallTofinoDriver(io::Error::other(format!(
            "pkg install exited with status {}",
            status
        ))));
    }

    // Enable the dendrite services with configuration
    enable_dendrite_services(front_ports, rear_ports, pkt_source)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum EnableDendriteServicesError {
    #[error("failed to enable dendrite service: {0}")]
    EnableDendrite(ScfError),

    #[error("failed to set dendrite service properties: {0}")]
    SetDendriteProperties(ScfError),

    #[error("failed to enable tfport service: {0}")]
    EnableTfport(ScfError),

    #[error("failed to set tfport service properties: {0}")]
    SetTfportProperties(ScfError),

    #[error("failed to enable uplink service: {0}")]
    EnableUplink(ScfError),
}

/// Enable the dendrite services `dendrite`, `tfport` and `uplink` using the smf crate.
pub fn enable_dendrite_services(
    front_ports: usize,
    rear_ports: usize,
    pkt_source: &str,
) -> Result<(), EnableDendriteServicesError> {
    use EnableDendriteServicesError as E;

    let scf = Scf::new().map_err(E::EnableDendrite)?;

    // Enable dendrite service and set properties
    let scope = scf.scope_local().map_err(E::EnableDendrite)?;
    let svc = scope
        .get_service("oxide/dendrite")
        .map_err(E::EnableDendrite)?
        .ok_or(E::EnableDendrite(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::EnableDendrite)?
        .ok_or(E::EnableDendrite(ScfError::NotFound))?;

    // Set SMF properties in the config property group
    let pg = inst
        .get_pg("config")
        .map_err(E::SetDendriteProperties)?
        .unwrap_or_else(|| {
            inst.add_pg("config", "application")
                .expect("failed to create config property group")
        });

    let tx = pg.transaction().map_err(E::SetDendriteProperties)?;
    tx.start().map_err(E::SetDendriteProperties)?;
    tx.property_ensure(
        "front_ports",
        scf_type_t::SCF_TYPE_ASTRING,
        &front_ports.to_string(),
    )
    .map_err(E::SetDendriteProperties)?;
    tx.property_ensure(
        "rear_ports",
        scf_type_t::SCF_TYPE_ASTRING,
        &rear_ports.to_string(),
    )
    .map_err(E::SetDendriteProperties)?;
    tx.property_ensure("mgmt", scf_type_t::SCF_TYPE_ASTRING, "uart")
        .map_err(E::SetDendriteProperties)?;
    tx.property_ensure("address", scf_type_t::SCF_TYPE_ASTRING, "[::]:12224")
        .map_err(E::SetDendriteProperties)?;
    tx.commit().map_err(E::SetDendriteProperties)?;

    // Refresh the instance to pick up the new configuration before enabling
    inst.refresh().map_err(E::EnableDendrite)?;

    inst.enable(false).map_err(E::EnableDendrite)?;

    // Enable tfport service and set properties
    let svc = scope
        .get_service("oxide/tfport")
        .map_err(E::EnableTfport)?
        .ok_or(E::EnableTfport(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::EnableTfport)?
        .ok_or(E::EnableTfport(ScfError::NotFound))?;

    // Set SMF properties in the config property group
    let pg = inst
        .get_pg("config")
        .map_err(E::SetTfportProperties)?
        .unwrap_or_else(|| {
            inst.add_pg("config", "application")
                .expect("failed to create config property group")
        });

    let tx = pg.transaction().map_err(E::SetTfportProperties)?;
    tx.start().map_err(E::SetTfportProperties)?;
    tx.property_ensure("pkt_source", scf_type_t::SCF_TYPE_ASTRING, pkt_source)
        .map_err(E::SetTfportProperties)?;
    tx.commit().map_err(E::SetTfportProperties)?;

    // Refresh the instance to pick up the new configuration before enabling
    inst.refresh().map_err(E::EnableTfport)?;

    inst.enable(false).map_err(E::EnableTfport)?;

    // Enable uplink service
    let svc = scope
        .get_service("oxide/uplink")
        .map_err(E::EnableUplink)?
        .ok_or(E::EnableUplink(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::EnableUplink)?
        .ok_or(E::EnableUplink(ScfError::NotFound))?;
    inst.enable(false).map_err(E::EnableUplink)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum DisableDendriteServicesError {
    #[error("failed to disable dendrite service: {0}")]
    DisableDendrite(ScfError),

    #[error("failed to disable tfport service: {0}")]
    DisableTfport(ScfError),

    #[error("failed to disable uplink service: {0}")]
    DisableUplink(ScfError),
}

/// Disables the dendrite services `dendrite`, `tfport` and `uplink` using the smf crate.
pub fn disable_dendrite_services() -> Result<(), DisableDendriteServicesError> {
    use DisableDendriteServicesError as E;

    let scf = Scf::new().map_err(E::DisableDendrite)?;

    // Disable dendrite service
    let scope = scf.scope_local().map_err(E::DisableDendrite)?;
    let svc = scope
        .get_service("oxide/dendrite")
        .map_err(E::DisableDendrite)?
        .ok_or(E::DisableDendrite(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::DisableDendrite)?
        .ok_or(E::DisableDendrite(ScfError::NotFound))?;
    inst.disable(false).map_err(E::DisableDendrite)?;

    // Disable tfport service
    let svc = scope
        .get_service("oxide/tfport")
        .map_err(E::DisableTfport)?
        .ok_or(E::DisableTfport(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::DisableTfport)?
        .ok_or(E::DisableTfport(ScfError::NotFound))?;
    inst.disable(false).map_err(E::DisableTfport)?;

    // Disable uplink service
    let svc = scope
        .get_service("oxide/uplink")
        .map_err(E::DisableUplink)?
        .ok_or(E::DisableUplink(ScfError::NotFound))?;
    let inst = svc
        .get_instance("default")
        .map_err(E::DisableUplink)?
        .ok_or(E::DisableUplink(ScfError::NotFound))?;
    inst.disable(false).map_err(E::DisableUplink)?;

    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum FetchImageError {
    #[error("failed to create temporary directory: {0}")]
    CreateTempDir(io::Error),

    #[error("failed to initialize curl: {0}")]
    CurlInit(curl::Error),

    #[error("failed to download image: {0}")]
    Download(curl::Error),

    #[error("HTTP error {0} when downloading image")]
    HttpError(u32),

    #[error("failed to write downloaded data: {0}")]
    WriteData(io::Error),

    #[error("failed to extract archive: {0}")]
    ExtractArchive(io::Error),

    #[error("extracted path is not valid UTF-8")]
    InvalidUtf8Path,

    #[error("failed to resolve version for branch {branch}: {error}")]
    ResolveVersion { branch: String, error: String },
}

#[derive(Debug, thiserror::Error)]
pub enum FetchArtifactsError {
    #[error("failed to initialize curl: {0}")]
    CurlInit(curl::Error),

    #[error("failed to download scadm: {0}")]
    DownloadScadm(DownloadError),

    #[error("failed to download libsidecar_lite.so: {0}")]
    DownloadLibsidecarLite(DownloadError),

    #[error("failed to write scadm: {0}")]
    WriteScadm(io::Error),

    #[error("failed to write libsidecar_lite.so: {0}")]
    WriteLibsidecarLite(io::Error),

    #[error("failed to set scadm executable permissions: {0}")]
    SetScadmExecutable(io::Error),

    #[error("failed to resolve version for branch {branch}: {error}")]
    ResolveVersion { branch: String, error: String },
}

#[derive(Debug, thiserror::Error)]
pub enum DownloadError {
    #[error("curl error: {0}")]
    Curl(curl::Error),

    #[error("HTTP error {0}")]
    HttpError(u32),

    #[error("failed to create file: {0}")]
    CreateFile(io::Error),
}

/// Helper function to download a file with retry logic and progress bar
fn download_file_with_retry(
    url: &str,
    dest_path: &std::path::Path,
    name: &str,
) -> Result<(), DownloadError> {
    use DownloadError as E;

    const MAX_RETRIES: u32 = 5;
    const RETRY_DELAY: Duration = Duration::from_secs(1);

    let pb = ProgressBar::new(0);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("valid template")
            .progress_chars("#>-"),
    );
    pb.set_message(name.to_string());

    let mut last_error = None;
    for attempt in 1..=MAX_RETRIES {
        let file = match fs::File::create(dest_path) {
            Ok(f) => f,
            Err(e) => {
                last_error = Some(E::CreateFile(e));
                if attempt < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                    continue;
                }
                break;
            }
        };

        // Reset progress bar for retry
        pb.set_position(0);

        let mut easy = Easy::new();
        if let Err(e) = easy.url(url) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.follow_location(true) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.connect_timeout(Duration::from_secs(30)) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.low_speed_limit(1000) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.low_speed_time(Duration::from_secs(30)) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.progress(true) {
            last_error = Some(E::Curl(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }

        let file = Arc::new(Mutex::new(file));
        let file_clone = Arc::clone(&file);

        let download_result = {
            let mut transfer = easy.transfer();

            // Set up progress callback
            let pb_ref = &pb;
            transfer
                .progress_function(move |dltotal, dlnow, _, _| {
                    if dltotal > 0.0 {
                        pb_ref.set_length(dltotal as u64);
                        pb_ref.set_position(dlnow as u64);
                    }
                    true
                })
                .map_err(E::Curl)?;

            transfer
                .write_function(move |data| {
                    let mut file = file_clone.lock().unwrap();
                    file.write_all(data)
                        .map_err(|_| curl::easy::WriteError::Pause)?;
                    Ok(data.len())
                })
                .map_err(E::Curl)?;

            transfer.perform()
        };

        match download_result {
            Ok(_) => {
                // Check HTTP response code
                match easy.response_code() {
                    Ok(200) => {
                        pb.finish_and_clear();
                        last_error = None;
                        break;
                    }
                    Ok(code) => {
                        last_error = Some(E::HttpError(code));
                        if attempt < MAX_RETRIES {
                            thread::sleep(RETRY_DELAY);
                        }
                    }
                    Err(e) => {
                        last_error = Some(E::Curl(e));
                        if attempt < MAX_RETRIES {
                            thread::sleep(RETRY_DELAY);
                        }
                    }
                }
            }
            Err(e) => {
                last_error = Some(E::Curl(e));
                if attempt < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
    }

    if let Some(err) = last_error {
        pb.finish_and_clear();
        return Err(err);
    }

    Ok(())
}

/// Fetch sidecar-lite.so and scadm artifacts from buildomat.
///
/// The following artifacts are fetched and placed in the current working directory
/// - https://buildomat.eng.oxide.computer/public/file/oxidecomputer/sidecar-lite/release/$version/scadm
/// - https://buildomat.eng.oxide.computer/public/file/oxidecomputer/sidecar-lite/release/$version/libsidecar_lite.so
pub async fn fetch_sidecar_lite_artifacts<'a>(
    version: ArtifactVersion<'a>,
) -> Result<(), FetchArtifactsError> {
    use FetchArtifactsError as E;

    // Resolve version string based on ArtifactVersion type
    let branch_to_resolve = match version {
        ArtifactVersion::Latest => Some("main"),
        ArtifactVersion::Commit(_) => None,
        ArtifactVersion::Branch(branch) => Some(branch),
    };

    let version_str = if let Some(branch) = branch_to_resolve {
        // Resolve branch to latest commit using octocrab
        let octocrab = octocrab::instance();
        let branch_info = octocrab
            .repos("oxidecomputer", "sidecar-lite")
            .get_ref(&octocrab::params::repos::Reference::Branch(
                branch.to_string(),
            ))
            .await
            .map_err(|e| E::ResolveVersion {
                branch: branch.to_string(),
                error: e.to_string(),
            })?;

        match branch_info.object {
            octocrab::models::repos::Object::Commit { sha, .. } => sha,
            octocrab::models::repos::Object::Tag { sha, .. } => sha,
            _ => {
                return Err(E::ResolveVersion {
                    branch: branch.to_string(),
                    error: "unexpected ref object type".to_string(),
                })
            }
        }
    } else if let ArtifactVersion::Commit(commit) = version {
        commit.to_string()
    } else {
        unreachable!()
    };

    // Build download URLs
    let scadm_url = format!(
        "https://buildomat.eng.oxide.computer/public/file/oxidecomputer/sidecar-lite/release/{}/scadm",
        version_str
    );
    let libsidecar_url = format!(
        "https://buildomat.eng.oxide.computer/public/file/oxidecomputer/sidecar-lite/release/{}/libsidecar_lite.so",
        version_str
    );

    // Download scadm
    let scadm_path = std::path::Path::new("scadm");
    download_file_with_retry(&scadm_url, scadm_path, "scadm")
        .map_err(E::DownloadScadm)?;

    // Set scadm as executable (chmod +x)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(scadm_path)
            .map_err(E::SetScadmExecutable)?
            .permissions();
        perms.set_mode(0o755);
        fs::set_permissions(scadm_path, perms)
            .map_err(E::SetScadmExecutable)?;
    }

    // Download libsidecar_lite.so
    let libsidecar_path = std::path::Path::new("libsidecar_lite.so");
    download_file_with_retry(&libsidecar_url, libsidecar_path, "libsidecar_lite.so")
        .map_err(E::DownloadLibsidecarLite)?;

    Ok(())
}

/// Fetch softnpu dendrite image from buildomat. Extract the archive and return
/// a path to the extracted archive.
///
/// The image is fetched from
/// https://buildomat.eng.oxide.computer/public/file/oxidecomputer/dendrite/image/$version/dendrite-softnpu.tar.gz
///
/// The package has a top level folder named `root` that contains a root
/// filesystem image.
pub async fn fetch_dendrite_image<'a>(
    version: ArtifactVersion<'a>,
) -> Result<Utf8PathBuf, FetchImageError> {
    use FetchImageError as E;

    // Resolve version string based on ArtifactVersion type
    let branch_to_resolve = match version {
        ArtifactVersion::Latest => Some("main"),
        ArtifactVersion::Commit(_) => None,
        ArtifactVersion::Branch(branch) => Some(branch),
    };

    let version_str = if let Some(branch) = branch_to_resolve {
        // Resolve branch to latest commit using octocrab
        let octocrab = octocrab::instance();
        let branch_info = octocrab
            .repos("oxidecomputer", "dendrite")
            .get_ref(&octocrab::params::repos::Reference::Branch(
                branch.to_string(),
            ))
            .await
            .map_err(|e| E::ResolveVersion {
                branch: branch.to_string(),
                error: e.to_string(),
            })?;

        match branch_info.object {
            octocrab::models::repos::Object::Commit { sha, .. } => sha,
            octocrab::models::repos::Object::Tag { sha, .. } => sha,
            _ => {
                return Err(E::ResolveVersion {
                    branch: branch.to_string(),
                    error: "unexpected ref object type".to_string(),
                })
            }
        }
    } else if let ArtifactVersion::Commit(commit) = version {
        commit.to_string()
    } else {
        unreachable!()
    };

    // Build download URL
    let url = format!(
        "https://buildomat.eng.oxide.computer/public/file/oxidecomputer/dendrite/image/{}/dendrite-softnpu.tar.gz",
        version_str
    );

    // Create temporary directory for extraction
    let temp_dir = TempDir::new().map_err(E::CreateTempDir)?;
    let archive_path = temp_dir.path().join("dendrite-softnpu.tar.gz");

    // Download the archive using curl with retry logic and progress bar
    const MAX_RETRIES: u32 = 5;
    const RETRY_DELAY: Duration = Duration::from_secs(1);

    let pb = ProgressBar::new(0);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("{msg} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
            .expect("valid template")
            .progress_chars("#>-"),
    );
    pb.set_message("dendrite-softnpu.tar.gz");

    let mut last_error = None;
    for attempt in 1..=MAX_RETRIES {
        let file = match fs::File::create(&archive_path) {
            Ok(f) => f,
            Err(e) => {
                last_error = Some(E::WriteData(e));
                if attempt < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                    continue;
                }
                break;
            }
        };

        // Reset progress bar for retry
        pb.set_position(0);

        let mut easy = Easy::new();
        if let Err(e) = easy.url(&url) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.follow_location(true) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        // Set connection timeout to 30 seconds
        if let Err(e) = easy.connect_timeout(Duration::from_secs(30)) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        // Abort if slower than 1000 bytes/sec during 30 seconds
        if let Err(e) = easy.low_speed_limit(1000) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.low_speed_time(Duration::from_secs(30)) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }
        if let Err(e) = easy.progress(true) {
            last_error = Some(E::CurlInit(e));
            if attempt < MAX_RETRIES {
                thread::sleep(RETRY_DELAY);
                continue;
            }
            break;
        }

        let file = Arc::new(Mutex::new(file));
        let file_clone = Arc::clone(&file);

        let download_result = {
            let mut transfer = easy.transfer();

            // Set up progress callback
            let pb_ref = &pb;
            transfer
                .progress_function(move |dltotal, dlnow, _, _| {
                    if dltotal > 0.0 {
                        pb_ref.set_length(dltotal as u64);
                        pb_ref.set_position(dlnow as u64);
                    }
                    true
                })
                .map_err(E::Download)?;

            transfer
                .write_function(move |data| {
                    let mut file = file_clone.lock().unwrap();
                    file.write_all(data)
                        .map_err(|_| curl::easy::WriteError::Pause)?;
                    Ok(data.len())
                })
                .map_err(E::Download)?;

            transfer.perform().map_err(E::Download)
        };

        match download_result {
            Ok(_) => {
                // Check HTTP response code
                let response_code =
                    easy.response_code().map_err(E::Download)?;
                if response_code == 200 {
                    // Download succeeded
                    pb.finish_and_clear();
                    last_error = None;
                    break;
                } else {
                    last_error = Some(E::HttpError(response_code));
                    if attempt < MAX_RETRIES {
                        thread::sleep(RETRY_DELAY);
                    }
                }
            }
            Err(e) => {
                last_error = Some(e);
                if attempt < MAX_RETRIES {
                    thread::sleep(RETRY_DELAY);
                }
            }
        }
    }

    if let Some(err) = last_error {
        pb.finish_and_clear();
        return Err(err);
    }

    // Extract the tar.gz archive
    let tar_gz = fs::File::open(&archive_path).map_err(E::ExtractArchive)?;
    let tar = GzDecoder::new(tar_gz);
    let mut archive = Archive::new(tar);
    archive.unpack(temp_dir.path()).map_err(E::ExtractArchive)?;

    // Convert path to Utf8PathBuf and return
    // The archive extracts to temp_dir/root
    let extracted_path = temp_dir.path().join("root");
    let utf8_path = Utf8PathBuf::from_path_buf(extracted_path)
        .map_err(|_| E::InvalidUtf8Path)?;

    // We need to leak the temp_dir to prevent it from being cleaned up
    std::mem::forget(temp_dir);

    Ok(utf8_path)
}
