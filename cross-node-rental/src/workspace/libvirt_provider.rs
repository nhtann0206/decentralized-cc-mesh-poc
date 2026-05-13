//! LibvirtQemuRuntime — ContainerRuntime implementation backed by libvirt + QEMU.
//!
//! Built from M1 findings (see `sprints/research-hub/research/gcp-nested-kvm-findings.md`).
//! Key invariants from that research:
//! - Debian 13 cloud image MUST boot via UEFI (`--boot uefi --machine q35`),
//!   or GRUB drops into a reboot loop.
//! - Tenant IPv6 reachability goes via a `socat` port-forward from the host's
//!   /128 public v6 address to the tenant's libvirt-NAT v4 address.
//! - `virsh suspend/resume` is the billing-loop pause primitive (~25ms round-trip).
//!
//! Host prerequisites:
//! - libvirt 11.3+ + QEMU 10+ (Debian trixie ships these).
//! - `qemu-img`, `virt-install`, `cloud-localds`, `socat` on PATH.
//! - OVMF firmware present (installed with `qemu-system-x86`).
//! - `/dev/kvm` accessible to the backend process.
//! - A Debian 13 base qcow2 image at `LIBVIRT_BASE_IMAGE` (default
//!   `/srv/tenants/base/debian-13-trixie.qcow2`).
//! - The host's public IPv6 in `LIBVIRT_HOST_IPV6` (for access URLs).
//! - Tenant SSH private key at `LIBVIRT_TENANT_KEY` (default
//!   `/root/.ssh/tenant_key`) for `exec_in_container`.

use async_trait::async_trait;
use std::collections::{HashMap, HashSet};
use std::process::Stdio;
use std::sync::Arc;
use tokio::process::Command;
use tokio::sync::Mutex;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use super::container::{ContainerConfig, ContainerRuntime, ContainerStatus};

/// Regex-equivalent validator for tenant domain names. Accepts only
/// `[a-zA-Z0-9_-]`, length 1..=64. Rejects any character that could
/// become a filesystem path traversal (`.`, `/`, `..`), shell metacharacter
/// (`;`, `|`, `` ` ``), or cloud-init YAML control (`\n`, `\r`, `:`, `#`).
/// Enforced at the `create_container` entry point before any disk write,
/// virt-install call, or cloud-init seed generation.
fn is_valid_tenant_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// Reject SSH public keys that are malformed OR contain newlines which would
/// let a renter inject additional YAML keys (e.g. a second `- name: root`
/// user block) into the generated cloud-init `user-data`. Accepts the three
/// common ed25519 / rsa / ecdsa shapes, optionally followed by a comment.
/// No `\n`, `\r`, or null byte anywhere in the key.
/// Build the `--disk` argument for the tenant overlay qcow2.
///
/// `cache=writethrough` is non-negotiable for Phase 2 home hardware: every
/// guest write must hit the qcow2 + flush host page cache before completing.
/// Default (writeback) leaves the overlay vulnerable to corruption on host
/// power-loss, which would cascade into LDK ChannelMonitor / tenant FS
/// inconsistency. Slight throughput cost; safety is non-optional.
///
/// Tested in `disk_overlay_arg_uses_writethrough`.
fn build_disk_overlay_arg(overlay_path: &str) -> String {
    format!(
        "path={},format=qcow2,bus=virtio,cache=writethrough",
        overlay_path
    )
}

fn is_valid_ssh_pubkey(key: &str) -> bool {
    if key.is_empty() || key.len() > 8192 {
        return false;
    }
    if key
        .bytes()
        .any(|b| b == b'\n' || b == b'\r' || b == 0)
    {
        return false;
    }
    let first_token = match key.split_whitespace().next() {
        Some(t) => t,
        None => return false,
    };
    matches!(
        first_token,
        "ssh-ed25519" | "ssh-rsa" | "ssh-dss"
    ) || first_token.starts_with("ecdsa-sha2-")
        || first_token.starts_with("sk-ssh-ed25519@")
        || first_token.starts_with("sk-ecdsa-sha2-")
}

const DEFAULT_BASE_IMAGE_PATH: &str = "/srv/tenants/base/debian-13-trixie.qcow2";
const DEFAULT_OVERLAY_DIR: &str = "/srv/tenants/overlays";
const DEFAULT_SEED_DIR: &str = "/srv/tenants/seeds";
const DEFAULT_SERIAL_LOG_DIR: &str = "/srv/tenants/serial-logs";
// Must be >= backing image virtual size. Debian cloud image is 20 GB virtual
// (disk sparse so actual on-disk is small). Smaller overlay truncates the GPT
// partition beyond overlay end — root partition inaccessible → initramfs panic
// "PARTUUID does not exist". qcow2 allocates only on-demand so no disk waste.
const TENANT_OVERLAY_SIZE: &str = "20G";
const DEFAULT_DHCP_POLL_TIMEOUT_SECS: u64 = 180;
const DHCP_POLL_INTERVAL_MS: u64 = 1000;
const V6_PORT_BASE: u16 = 2200;
const V6_PORT_RANGE: u16 = 100;

// Per-call shell-out timeouts (seconds). Without these, a hung virt-install /
// virsh would wedge a workspace task forever. Picked from M1 observed bench:
// virsh control-plane calls finish in <1s, qemu-img create in <1s on pd-ssd,
// virt-install takes ~5–15s to define a domain with UEFI firmware copy, and
// ssh exec should round-trip in <2s on a healthy tenant. Doubled for safety.
const VIRSH_TIMEOUT_SECS: u64 = 10;
const QEMU_IMG_TIMEOUT_SECS: u64 = 60;
const CLOUD_LOCALDS_TIMEOUT_SECS: u64 = 30;
const VIRT_INSTALL_TIMEOUT_SECS: u64 = 120;
const PREFLIGHT_TIMEOUT_SECS: u64 = 5;
const SSH_EXEC_TIMEOUT_SECS: u64 = 30;

/// Run an external command with a wall-clock timeout. Returns the process
/// output on success. Any of (spawn failure, non-success exit, timeout) is
/// converted to a human-readable error string so callers can log + surface.
async fn run_cmd_with_timeout(
    program: &str,
    args: &[&str],
    timeout_secs: u64,
    what: &str,
) -> Result<std::process::Output, String> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    match tokio::time::timeout(
        std::time::Duration::from_secs(timeout_secs),
        cmd.output(),
    )
    .await
    {
        Ok(Ok(out)) => Ok(out),
        Ok(Err(e)) => Err(format!("{} spawn: {}", what, e)),
        Err(_) => Err(format!(
            "{} timed out after {}s",
            what, timeout_secs
        )),
    }
}

/// Per-tenant runtime state held in-memory by the backend process. On backend
/// crash this is lost — rehydration from libvirt is a Phase 2 concern (see
/// plan §6.6 crash recovery).
#[derive(Debug)]
struct TenantState {
    host_port: u16,
    tenant_ip: Option<String>,
    socat_pid: Option<u32>,
    overlay_path: String,
    seed_path: String,
    serial_log_path: String,
    /// QEMU VNC WebSocket port (e.g. 5700). Read from `virsh dumpxml`
    /// after VM start. Used by noVNC proxy to connect to VM desktop.
    vnc_ws_port: Option<u16>,
}

pub struct LibvirtQemuRuntime {
    base_image_path: String,
    host_ipv6: String,
    tenant_key_path: String,
    overlay_dir: String,
    seed_dir: String,
    serial_log_dir: String,
    dhcp_timeout_secs: u64,
    tenants: Arc<Mutex<HashMap<String, TenantState>>>,
    /// Set of currently-allocated host socat forward ports. Replaces the old
    /// monotonic counter so that: (a) port 2200-2299 are never double-used
    /// across concurrent tenants, (b) on `create_container` failure the port
    /// is returned to the pool deterministically, and (c) when a tenant
    /// releases its port (via `remove_container`) it becomes available for
    /// reuse.
    allocated_ports: Arc<Mutex<HashSet<u16>>>,
    /// Global lifecycle serialization mutex. Every create / start / stop /
    /// remove operation MUST hold this for its libvirt-touching section.
    /// Reason: RH bugzilla 1898190 + 1150505 document real races in
    /// parallel virt-install / virsh destroy / virsh undefine against
    /// libvirt's global state (nwfilter ebtables apply, network dhcp-host
    /// update, NVRAM template copy). Pause / resume operations are safe
    /// to run concurrently — they don't touch global state — so they
    /// deliberately do NOT take this lock.
    lifecycle_lock: Arc<Mutex<()>>,
}

impl LibvirtQemuRuntime {
    /// Construct the runtime from environment variables and verify host
    /// prerequisites. Returns `Err` with a human-readable message if any
    /// prerequisite is missing — the factory in `detect_runtime` will log
    /// and fall through to the next runtime option.
    pub async fn from_env() -> Result<Self, String> {
        let base_image = std::env::var("LIBVIRT_BASE_IMAGE")
            .unwrap_or_else(|_| DEFAULT_BASE_IMAGE_PATH.to_string());
        let host_ipv6 = std::env::var("LIBVIRT_HOST_IPV6").map_err(|_| {
            "LIBVIRT_HOST_IPV6 env var not set — required to build tenant access URLs".to_string()
        })?;
        let tenant_key_path = std::env::var("LIBVIRT_TENANT_KEY")
            .unwrap_or_else(|_| "/root/.ssh/tenant_key".to_string());
        let overlay_dir = std::env::var("LIBVIRT_OVERLAY_DIR")
            .unwrap_or_else(|_| DEFAULT_OVERLAY_DIR.to_string());
        let seed_dir = std::env::var("LIBVIRT_SEED_DIR")
            .unwrap_or_else(|_| DEFAULT_SEED_DIR.to_string());
        let serial_log_dir = std::env::var("LIBVIRT_SERIAL_LOG_DIR")
            .unwrap_or_else(|_| DEFAULT_SERIAL_LOG_DIR.to_string());
        let dhcp_timeout_secs: u64 = std::env::var("LIBVIRT_DHCP_TIMEOUT_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(DEFAULT_DHCP_POLL_TIMEOUT_SECS);
        // LIBVIRT_SKIP_PREFLIGHT=1 skips /dev/kvm + tool version checks.
        // Only for stress-test harnesses where shell-outs are faked via PATH
        // injection and a real hypervisor is not present. Must NEVER be set
        // on production hosts.
        let skip_preflight = std::env::var("LIBVIRT_SKIP_PREFLIGHT")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        let virsh_version = if skip_preflight {
            warn!(
                target: "node_backend::workspace",
                "LIBVIRT_SKIP_PREFLIGHT=1 — skipping /dev/kvm + tool version checks"
            );
            "skipped".to_string()
        } else {
            // Verify /dev/kvm
            if !std::path::Path::new("/dev/kvm").exists() {
                return Err("/dev/kvm not present; nested virt unavailable".to_string());
            }

            // Verify virsh on PATH
            let out = run_cmd_with_timeout(
                "virsh",
                &["--version"],
                PREFLIGHT_TIMEOUT_SECS,
                "virsh --version",
            )
            .await?;
            if !out.status.success() {
                return Err("`virsh --version` failed".to_string());
            }
            let virsh_version = String::from_utf8_lossy(&out.stdout).trim().to_string();

            // Verify the other tools we shell out to
            for tool in &["qemu-img", "virt-install", "cloud-localds", "socat"] {
                let out = run_cmd_with_timeout(
                    tool,
                    &["--version"],
                    PREFLIGHT_TIMEOUT_SECS,
                    &format!("{} --version", tool),
                )
                .await?;
                if !out.status.success() {
                    // cloud-localds prints usage to stderr with exit 1 when given --version;
                    // accept any exit as long as the binary exists and ran.
                    debug!(
                        target: "node_backend::workspace",
                        tool = %tool,
                        "tool found but --version returned non-zero (may be benign)"
                    );
                }
            }
            virsh_version
        };

        // Verify base image present
        if !std::path::Path::new(&base_image).exists() {
            return Err(format!("base image not found at {}", base_image));
        }

        // Ensure runtime directories exist
        for d in &[overlay_dir.as_str(), seed_dir.as_str(), serial_log_dir.as_str()] {
            tokio::fs::create_dir_all(d)
                .await
                .map_err(|e| format!("failed to create {}: {}", d, e))?;
        }

        info!(
            target: "node_backend::workspace",
            virsh_version = %virsh_version,
            base_image = %base_image,
            host_ipv6 = %host_ipv6,
            overlay_dir = %overlay_dir,
            "LibvirtQemuRuntime initialized"
        );

        Ok(Self {
            base_image_path: base_image,
            host_ipv6,
            tenant_key_path,
            overlay_dir,
            seed_dir,
            serial_log_dir,
            dhcp_timeout_secs,
            tenants: Arc::new(Mutex::new(HashMap::new())),
            allocated_ports: Arc::new(Mutex::new(HashSet::new())),
            lifecycle_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Reserve an unused port in the tenant-forward range `[V6_PORT_BASE,
    /// V6_PORT_BASE + V6_PORT_RANGE)`. Returns the port on success, or an
    /// error string if the whole range is exhausted (caller must surface this
    /// to the service layer as "no tenant slot available"). Thread-safe: the
    /// `HashSet` is mutex-guarded and the insert happens inside the same
    /// lock as the scan, so two concurrent callers cannot race the same
    /// unused port.
    async fn allocate_port(&self) -> Result<u16, String> {
        let mut ports = self.allocated_ports.lock().await;
        for offset in 0..V6_PORT_RANGE {
            let candidate = V6_PORT_BASE + offset;
            if ports.insert(candidate) {
                return Ok(candidate);
            }
        }
        Err(format!(
            "tenant port range exhausted ({} slots, {}..{})",
            V6_PORT_RANGE,
            V6_PORT_BASE,
            V6_PORT_BASE + V6_PORT_RANGE - 1
        ))
    }

    /// Return a port to the pool. Called on `create_container` error
    /// rollback and on `remove_container` success. Idempotent: safe to call
    /// even if the port was already released (remove returns false).
    async fn release_port(&self, port: u16) {
        let mut ports = self.allocated_ports.lock().await;
        ports.remove(&port);
    }

    async fn create_overlay(&self, name: &str) -> Result<String, String> {
        let overlay = format!("{}/{}.qcow2", self.overlay_dir, name);
        let out = run_cmd_with_timeout(
            "qemu-img",
            &[
                "create",
                "-f",
                "qcow2",
                "-F",
                "qcow2",
                "-b",
                self.base_image_path.as_str(),
                overlay.as_str(),
                TENANT_OVERLAY_SIZE,
            ],
            QEMU_IMG_TIMEOUT_SECS,
            "qemu-img create",
        )
        .await?;
        if !out.status.success() {
            return Err(format!(
                "qemu-img create failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        debug!(
            target: "node_backend::workspace",
            overlay = %overlay,
            base = %self.base_image_path,
            "qcow2 overlay created"
        );
        Ok(overlay)
    }

    async fn create_cloud_init_seed(&self, name: &str, pubkey: &str) -> Result<String, String> {
        let seed = format!("{}/{}-seed.iso", self.seed_dir, name);

        // Stage user-data + meta-data in a per-tenant temp dir under the seed
        // dir so cleanup is deterministic (no reliance on the tempfile crate).
        let workdir = format!("{}/{}-stage", self.seed_dir, name);
        tokio::fs::create_dir_all(&workdir)
            .await
            .map_err(|e| format!("mkdir stage: {}", e))?;
        let user_data_path = format!("{}/user-data", workdir);
        let meta_data_path = format!("{}/meta-data", workdir);

        let user_data = format!(
            "#cloud-config\n\
             hostname: {name}\n\
             users:\n\
             \x20\x20- name: tenant\n\
             \x20\x20\x20\x20sudo: ALL=(ALL) NOPASSWD:ALL\n\
             \x20\x20\x20\x20shell: /bin/bash\n\
             \x20\x20\x20\x20ssh_authorized_keys:\n\
             \x20\x20\x20\x20\x20\x20- {pubkey}\n\
             ssh_pwauth: false\n\
             package_update: false\n",
            name = name,
            pubkey = pubkey
        );
        let meta_data = format!(
            "instance-id: {name}\nlocal-hostname: {name}\n",
            name = name
        );

        // Helper: always drop the per-tenant stage dir on every return path,
        // whether user-data write, meta-data write, or cloud-localds fails.
        // Without this the `{seed_dir}/{name}-stage/` dirs accumulate on
        // every failed create.
        let cleanup_stage = |wd: &str| {
            let wd = wd.to_string();
            async move {
                let _ = tokio::fs::remove_dir_all(&wd).await;
            }
        };

        if let Err(e) = tokio::fs::write(&user_data_path, user_data).await {
            cleanup_stage(&workdir).await;
            return Err(format!("write user-data: {}", e));
        }
        if let Err(e) = tokio::fs::write(&meta_data_path, meta_data).await {
            cleanup_stage(&workdir).await;
            return Err(format!("write meta-data: {}", e));
        }

        let out = match run_cmd_with_timeout(
            "cloud-localds",
            &[
                seed.as_str(),
                user_data_path.as_str(),
                meta_data_path.as_str(),
            ],
            CLOUD_LOCALDS_TIMEOUT_SECS,
            "cloud-localds",
        )
        .await
        {
            Ok(o) => o,
            Err(e) => {
                cleanup_stage(&workdir).await;
                return Err(e);
            }
        };
        if !out.status.success() {
            cleanup_stage(&workdir).await;
            return Err(format!(
                "cloud-localds failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }

        cleanup_stage(&workdir).await;

        debug!(
            target: "node_backend::workspace",
            seed = %seed,
            "cloud-init seed ISO generated"
        );
        Ok(seed)
    }

    async fn virt_install(
        &self,
        name: &str,
        ram_mb: u32,
        overlay: &str,
        seed: &str,
        serial_log: &str,
    ) -> Result<(), String> {
        // virt-install with --import + --noreboot defines the domain via
        // libvirt and starts it; it does not block on guest OS completion.
        // The DHCP wait downstream handles readiness.
        //
        // CPU config: host-passthrough + explicit Spectre-family side-channel
        // features. The L0 (GCP) kernel applies its own mitigations, but
        // those protect GCP's own multi-tenancy — they do NOT automatically
        // propagate into nested L2 guests. VMScape (ETH Zürich S&P 2026)
        // and corCTF 2024 "Trojan Turtles" demonstrated practical cross-L2
        // leaks on nested KVM when these MSRs aren't passed through. We
        // require spec-ctrl, ssbd, md-clear, ibpb so the guest kernel can
        // enable its own Spectre-v2 / MDS / SSBD mitigations.
        //
        // Network: `filterref=clean-traffic` attaches libvirt's built-in
        // nwfilter that blocks MAC spoofing, IP spoofing, and ARP poisoning
        // on the shared virbr0 bridge. Without it, tenant A can ARP-poison
        // tenant B on the L2 broadcast domain (libvirt default NAT does
        // NOT isolate tenants). `<port isolated='yes'/>` on the network
        // itself is an operator-side prerequisite — documented in Phase 1
        // operator runbook — because we cannot safely redefine the default
        // libvirt network at runtime without kicking off live guests.
        let ram_str = ram_mb.to_string();
        let disk_overlay = build_disk_overlay_arg(&overlay);
        let disk_seed = format!("path={},device=cdrom", seed);
        let serial_arg = format!("file,path={}", serial_log);
        let cpu_arg = "host-passthrough,check=partial,\
                       +spec-ctrl,+ssbd,+md-clear,+ibpb,+amd-ssbd,+amd-no-ssb";
        let network_arg = "network=default,model=virtio,filterref=clean-traffic";
        let out = run_cmd_with_timeout(
            "virt-install",
            &[
                "--name",
                name,
                "--memory",
                ram_str.as_str(),
                "--vcpus",
                "1",
                "--cpu",
                cpu_arg,
                "--disk",
                disk_overlay.as_str(),
                "--disk",
                disk_seed.as_str(),
                "--os-variant",
                "debian13",
                "--boot",
                "uefi",
                "--machine",
                "q35",
                "--network",
                network_arg,
                "--graphics",
                "vnc,listen=127.0.0.1,websocket=-1",
                // Absolute pointing device so browser-canvas click
                // coordinates map 1:1 to the guest. Without this, QEMU
                // defaults to PS/2 mouse (relative coordinates) and the
                // VNC proxy delivers clicks that land at the wrong spot
                // on the guest desktop.
                "--input",
                "tablet,bus=usb",
                "--serial",
                serial_arg.as_str(),
                "--noautoconsole",
                "--import",
                "--noreboot",
            ],
            VIRT_INSTALL_TIMEOUT_SECS,
            "virt-install",
        )
        .await?;
        if !out.status.success() {
            return Err(format!(
                "virt-install failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(())
    }

    async fn wait_for_dhcp_lease(&self, name: &str) -> Result<String, String> {
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_secs(self.dhcp_timeout_secs);
        while std::time::Instant::now() < deadline {
            let out = run_cmd_with_timeout(
                "virsh",
                &["net-dhcp-leases", "default"],
                VIRSH_TIMEOUT_SECS,
                "virsh net-dhcp-leases",
            )
            .await?;
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                for line in text.lines() {
                    if !line.contains(name) {
                        continue;
                    }
                    // Table row looks like:
                    //   "2026-04-15 03:55  52:54:xx  ipv4  192.168.122.147/24  tenant-1  ..."
                    for token in line.split_whitespace() {
                        let ip = token.split('/').next().unwrap_or("");
                        if ip.chars().filter(|c| *c == '.').count() == 3
                            && ip.chars().all(|c| c.is_ascii_digit() || c == '.')
                        {
                            return Ok(ip.to_string());
                        }
                    }
                }
            } else {
                warn!(
                    target: "node_backend::workspace",
                    stderr = %String::from_utf8_lossy(&out.stderr),
                    "virsh net-dhcp-leases returned non-zero"
                );
            }
            tokio::time::sleep(std::time::Duration::from_millis(DHCP_POLL_INTERVAL_MS)).await;
        }
        Err(format!(
            "tenant {} did not acquire a DHCP lease within {}s",
            name, self.dhcp_timeout_secs
        ))
    }

    async fn start_socat_forward(&self, port: u16, tenant_ip: &str) -> Result<u32, String> {
        // tokio::process::Child default drop does NOT kill the child; we let
        // socat run detached and track it via PID for later teardown.
        let child = Command::new("socat")
            .args([
                &format!("TCP6-LISTEN:{},fork,reuseaddr", port),
                &format!("TCP4:{}:22", tenant_ip),
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("socat spawn: {}", e))?;
        let pid = child.id().ok_or_else(|| "socat has no pid".to_string())?;
        drop(child);
        debug!(
            target: "node_backend::workspace",
            pid = pid,
            port = port,
            tenant_ip = %tenant_ip,
            "socat forward started"
        );
        Ok(pid)
    }

    /// Return the on-host path of the tenant's QEMU serial log, if the tenant
    /// is still tracked. Useful for post-mortem debugging when a tenant fails
    /// to boot (captured via `--serial file,path=...` on virt-install).
    pub async fn serial_log_path(&self, container_id: &str) -> Option<String> {
        let tenants = self.tenants.lock().await;
        tenants
            .get(container_id)
            .map(|s| s.serial_log_path.clone())
    }

    async fn kill_pid(&self, pid: u32) {
        let pid_str = pid.to_string();
        if let Err(e) = run_cmd_with_timeout(
            "kill",
            &[pid_str.as_str()],
            PREFLIGHT_TIMEOUT_SECS,
            "kill",
        )
        .await
        {
            warn!(
                target: "node_backend::workspace",
                pid = pid,
                error = %e,
                "failed to kill pid"
            );
        }
    }

    /// Get the VNC WebSocket port for a running tenant. Returns None if
    /// the tenant doesn't exist or VNC is not configured.
    pub async fn get_vnc_ws_port(&self, container_id: &str) -> Option<u16> {
        let tenants = self.tenants.lock().await;
        tenants.get(container_id).and_then(|s| s.vnc_ws_port)
    }

    /// Parse VNC WebSocket port from `virsh dumpxml <domain>`. The
    /// `--graphics vnc,websocket=-1` flag tells QEMU to auto-assign a
    /// WebSocket port. After the VM starts, the actual port appears in
    /// the domain XML: `<graphics type='vnc' port='5900' websocket='5700'/>`.
    async fn read_vnc_ws_port(&self, container_id: &str) -> Option<u16> {
        let out = run_cmd_with_timeout(
            "virsh",
            &["dumpxml", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh dumpxml (vnc port)",
        )
        .await
        .ok()?;
        if !out.status.success() {
            return None;
        }
        let xml = String::from_utf8_lossy(&out.stdout);
        // Parse: websocket='5700' from <graphics type='vnc' ... websocket='NNNN'/>
        for line in xml.lines() {
            if line.contains("type='vnc'") || line.contains("type=\"vnc\"") {
                if let Some(ws_start) = line.find("websocket='") {
                    let rest = &line[ws_start + 11..];
                    if let Some(ws_end) = rest.find('\'') {
                        if let Ok(port) = rest[..ws_end].parse::<u16>() {
                            debug!(
                                target: "node_backend::workspace",
                                tenant = %container_id,
                                vnc_ws_port = port,
                                "Read VNC WebSocket port from domain XML"
                            );
                            return Some(port);
                        }
                    }
                }
                // Also try double-quote variant
                if let Some(ws_start) = line.find("websocket=\"") {
                    let rest = &line[ws_start + 11..];
                    if let Some(ws_end) = rest.find('"') {
                        if let Ok(port) = rest[..ws_end].parse::<u16>() {
                            return Some(port);
                        }
                    }
                }
            }
        }
        warn!(
            target: "node_backend::workspace",
            tenant = %container_id,
            "Could not parse VNC WebSocket port from domain XML"
        );
        None
    }

    /// Read the raw `virsh domstate` output for a domain. Returns the
    /// lowercase trimmed state string (e.g. "running", "paused", "shut off").
    /// Callers compare via `==` on known strings.
    async fn read_domstate(&self, container_id: &str) -> Result<String, String> {
        let out = run_cmd_with_timeout(
            "virsh",
            &["domstate", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh domstate",
        )
        .await?;
        if !out.status.success() {
            return Err(format!(
                "virsh domstate failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_lowercase())
    }

    /// Debounced confirmation that a domain has reached a target state.
    /// Polls `virsh domstate` with `interval_ms` between reads and requires
    /// `consecutive` reads to return `target` consecutively before accepting.
    /// Overall budget is `deadline_ms`; if exceeded returns an error.
    ///
    /// Rationale: `virsh domstate` can return stale state for a sub-second
    /// window under libvirtd lock contention (see community.libvirt#63 +
    /// autotest/virt-test#198). Single-read confirm can false-positive or
    /// false-negative. Two consecutive matches is a pragmatic debounce.
    async fn confirm_domstate_debounced(
        &self,
        container_id: &str,
        target: &str,
        consecutive: u32,
        deadline_ms: u64,
        interval_ms: u64,
    ) -> Result<(), String> {
        let start = std::time::Instant::now();
        let deadline = start + std::time::Duration::from_millis(deadline_ms);
        let mut streak: u32 = 0;
        let mut last_observed: String = String::new();
        while std::time::Instant::now() < deadline {
            match self.read_domstate(container_id).await {
                Ok(state) => {
                    last_observed = state.clone();
                    if state == target {
                        streak += 1;
                        if streak >= consecutive {
                            return Ok(());
                        }
                    } else {
                        streak = 0;
                    }
                }
                Err(e) => {
                    warn!(
                        target: "node_backend::workspace",
                        tenant = %container_id,
                        error = %e,
                        "domstate read error during debounce, continuing"
                    );
                    streak = 0;
                }
            }
            tokio::time::sleep(std::time::Duration::from_millis(interval_ms)).await;
        }
        Err(format!(
            "domstate confirmation timeout for tenant {}: wanted {} × {}, last observed {}",
            container_id, target, consecutive, last_observed
        ))
    }

    /// The actual stop logic without taking the lifecycle lock. `stop_container`
    /// wraps this with the lock; `remove_container` calls this directly
    /// because it already holds the lifecycle lock for the combined
    /// stop+undefine operation. `tokio::sync::Mutex` is not reentrant, so
    /// we must not re-acquire the lock from within a held section.
    async fn stop_locked(&self, container_id: &str) -> Result<(), String> {
        // Kill socat first so port frees up cleanly.
        let socat_pid = {
            let tenants = self.tenants.lock().await;
            tenants.get(container_id).and_then(|s| s.socat_pid)
        };
        if let Some(pid) = socat_pid {
            self.kill_pid(pid).await;
        }

        let out = run_cmd_with_timeout(
            "virsh",
            &["destroy", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh destroy",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.contains("not running") && !stderr.contains("not found") {
                return Err(format!("virsh destroy failed: {}", stderr.trim()));
            }
        }

        {
            let mut tenants = self.tenants.lock().await;
            if let Some(state) = tenants.get_mut(container_id) {
                state.socat_pid = None;
                state.tenant_ip = None;
            }
        }

        info!(
            target: "node_backend::workspace",
            tenant = %container_id,
            "libvirt tenant stopped"
        );
        Ok(())
    }
}

#[async_trait]
impl ContainerRuntime for LibvirtQemuRuntime {
    async fn create_container(&self, config: ContainerConfig) -> Result<String, String> {
        // Hold the global lifecycle lock for the entire create operation.
        // Libvirt has documented races in parallel define / start / destroy /
        // undefine (RH bugzilla 1898190, 1150505): nwfilter ebtables apply,
        // network dhcp-host update, and NVRAM template copy all serialize
        // on global libvirtd mutexes and can deadlock under concurrent
        // stress. Accepting the latency cost of serialization is cheaper
        // than debugging a hung libvirtd on a real Phase 1 demo.
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        let pubkey = config
            .environment
            .get("SSH_AUTHORIZED_KEY")
            .cloned()
            .ok_or_else(|| {
                "SSH_AUTHORIZED_KEY missing from ContainerConfig.environment — \
                 libvirt tenants need a public key for cloud-init authorized_keys"
                    .to_string()
            })?;

        // Security: validate the ssh public key. A renter-supplied newline
        // would let us YAML-inject additional users into the generated
        // cloud-init user-data (e.g. a second `- name: root ...` block).
        if !is_valid_ssh_pubkey(&pubkey) {
            return Err(
                "SSH_AUTHORIZED_KEY is not a well-formed public key (must be one of \
                 ssh-ed25519 / ssh-rsa / ssh-dss / ecdsa-sha2-* / sk-* followed by a \
                 base64 blob, on a single line, no newlines or control bytes)"
                    .to_string(),
            );
        }

        let name = match config.name.as_ref() {
            Some(n) if !n.is_empty() => n.clone(),
            _ => {
                let short = Uuid::new_v4().simple().to_string()[..8].to_string();
                format!("tenant-{}", short)
            }
        };

        // Security: validate the tenant name against a strict charset before
        // it flows into libvirt domain name, filesystem paths (overlay,
        // seed, serial log), and cloud-init hostname. Without this, a
        // caller-supplied name like `../../etc/passwd` would traverse out of
        // `self.overlay_dir` on the `create_overlay` call below.
        if !is_valid_tenant_name(&name) {
            return Err(format!(
                "tenant name '{}' is invalid — must be 1..=64 chars of [a-zA-Z0-9_-]",
                name
            ));
        }

        // Uniqueness check + reservation: take the tenants lock once, confirm
        // this name is not already in use, and leave the lock released
        // before any slow shell-out. Two concurrent callers with the same
        // `config.name` will both reach here; the second one sees the first's
        // entry and fails fast, rather than silently overwriting the first's
        // TenantState and leaking its overlay.
        {
            let tenants = self.tenants.lock().await;
            if tenants.contains_key(&name) {
                return Err(format!("tenant '{}' already exists", name));
            }
        }

        // Reserve the host port first — if the whole forward range is full,
        // we want to fail before touching disk so there's nothing to clean
        // up. On any later failure in this function we MUST release_port().
        let host_port = self.allocate_port().await?;

        let overlay = match self.create_overlay(&name).await {
            Ok(p) => p,
            Err(e) => {
                self.release_port(host_port).await;
                return Err(e);
            }
        };

        let seed = match self.create_cloud_init_seed(&name, &pubkey).await {
            Ok(p) => p,
            Err(e) => {
                let _ = tokio::fs::remove_file(&overlay).await;
                self.release_port(host_port).await;
                return Err(e);
            }
        };

        let serial_log = format!("{}/{}.log", self.serial_log_dir, name);
        if let Err(e) = tokio::fs::write(&serial_log, "").await {
            let _ = tokio::fs::remove_file(&overlay).await;
            let _ = tokio::fs::remove_file(&seed).await;
            self.release_port(host_port).await;
            return Err(format!("init serial log: {}", e));
        }

        if let Err(e) = self
            .virt_install(&name, config.ram_mb, &overlay, &seed, &serial_log)
            .await
        {
            // Full rollback. virt-install --import --noreboot may have
            // already registered a partial domain definition with libvirt
            // (UEFI firmware copy can fail mid-flight), so best-effort
            // undefine before removing the overlay + seed + serial log.
            // If we skip the undefine the next create_container with the
            // same name hits "domain already exists" from libvirt.
            let _ = run_cmd_with_timeout(
                "virsh",
                &["undefine", "--nvram", name.as_str()],
                VIRSH_TIMEOUT_SECS,
                "virsh undefine (create rollback)",
            )
            .await;
            let _ = tokio::fs::remove_file(&overlay).await;
            let _ = tokio::fs::remove_file(&seed).await;
            let _ = tokio::fs::remove_file(&serial_log).await;
            self.release_port(host_port).await;
            error!(
                target: "node_backend::workspace",
                tenant = %name,
                error = %e,
                "virt-install failed, rolled back overlay+seed+serial+port"
            );
            return Err(e);
        }

        // Commit to tenants map. There is a small window between the
        // successful virt-install and this insert where another caller
        // could also pass the uniqueness check above for the same name
        // (read-side race), but the libvirt domain definition is now the
        // canonical source of truth; a racing second create would fail at
        // the virt-install step because libvirt rejects duplicate names.
        let mut tenants = self.tenants.lock().await;
        tenants.insert(
            name.clone(),
            TenantState {
                host_port,
                tenant_ip: None,
                socat_pid: None,
                overlay_path: overlay,
                seed_path: seed,
                serial_log_path: serial_log,
                vnc_ws_port: None, // set after VM start via virsh dumpxml
            },
        );

        info!(
            target: "node_backend::workspace",
            tenant = %name,
            ram_mb = config.ram_mb,
            host_port = host_port,
            "libvirt tenant defined via virt-install"
        );

        Ok(name)
    }

    async fn start_container(&self, container_id: &str) -> Result<(), String> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;

        // virt-install already starts the domain in create_container; this
        // function handles the idempotent case where the caller explicitly
        // transitions create → start, and the recovery case where a prior
        // stop_container destroyed the domain.
        let out = run_cmd_with_timeout(
            "virsh",
            &["start", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh start",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.contains("already active") {
                return Err(format!("virsh start failed: {}", stderr.trim()));
            }
        }

        let ip = self.wait_for_dhcp_lease(container_id).await?;

        // Resolve the host port from state. If the tenant was removed
        // between create_container and start_container, fail fast — do not
        // spawn a socat forward that would immediately be orphaned.
        let host_port = {
            let tenants = self.tenants.lock().await;
            tenants
                .get(container_id)
                .map(|t| t.host_port)
                .ok_or_else(|| {
                    format!("tenant {} not tracked in runtime state", container_id)
                })?
        };

        let socat_pid = self.start_socat_forward(host_port, &ip).await?;

        // Commit the socat PID + tenant IP under the tenants lock.
        // Critical: if `remove_container` ran concurrently between the lock
        // read above and this lock acquisition, the tenant entry no longer
        // exists. In that case we MUST kill the socat we just spawned,
        // otherwise it becomes an orphaned process holding the host port.
        // Read VNC WebSocket port from domain XML (auto-assigned by QEMU)
        let vnc_ws_port = self.read_vnc_ws_port(container_id).await;

        let race_detected = {
            let mut tenants = self.tenants.lock().await;
            match tenants.get_mut(container_id) {
                Some(state) => {
                    state.tenant_ip = Some(ip.clone());
                    state.socat_pid = Some(socat_pid);
                    state.vnc_ws_port = vnc_ws_port;
                    false
                }
                None => true,
            }
        };
        if race_detected {
            warn!(
                target: "node_backend::workspace",
                tenant = %container_id,
                socat_pid = socat_pid,
                "tenant was removed mid-start; killing orphaned socat forward"
            );
            self.kill_pid(socat_pid).await;
            self.release_port(host_port).await;
            return Err(format!(
                "tenant {} was removed during start_container",
                container_id
            ));
        }

        info!(
            target: "node_backend::workspace",
            tenant = %container_id,
            ip = %ip,
            host_port = host_port,
            socat_pid = socat_pid,
            "libvirt tenant started"
        );
        Ok(())
    }

    async fn stop_container(&self, container_id: &str) -> Result<(), String> {
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        self.stop_locked(container_id).await
    }

    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        // Single lifecycle lock acquisition for both stop + undefine. Calling
        // self.stop_container() here would take the lock recursively and
        // deadlock (tokio::sync::Mutex is NOT reentrant), so we go through
        // the un-locked internal helper.
        let _lifecycle_guard = self.lifecycle_lock.lock().await;
        let _ = self.stop_locked(container_id).await;

        // --nvram drops the per-domain OVMF NVRAM file (required because we
        // booted UEFI); without it, undefine fails on UEFI-backed domains.
        let out = run_cmd_with_timeout(
            "virsh",
            &["undefine", "--nvram", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh undefine",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if !stderr.contains("not found") && !stderr.contains("Domain not found") {
                return Err(format!("virsh undefine failed: {}", stderr.trim()));
            }
        }

        let removed_state = {
            let mut tenants = self.tenants.lock().await;
            tenants.remove(container_id)
        };
        if let Some(state) = removed_state {
            let _ = tokio::fs::remove_file(&state.overlay_path).await;
            let _ = tokio::fs::remove_file(&state.seed_path).await;
            // Serial log is retained for post-mortem debugging.
            // Return the host port to the allocator pool so a later tenant
            // can reuse it; without this the pool drifts toward exhaustion
            // over the Phase 1 demo lifetime.
            self.release_port(state.host_port).await;
        }

        info!(
            target: "node_backend::workspace",
            tenant = %container_id,
            "libvirt tenant removed"
        );
        Ok(())
    }

    async fn get_container_status(
        &self,
        container_id: &str,
    ) -> Result<ContainerStatus, String> {
        let out = run_cmd_with_timeout(
            "virsh",
            &["domstate", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh domstate",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if stderr.contains("not found") || stderr.contains("Domain not found") {
                return Ok(ContainerStatus::Removed);
            }
            return Err(format!("virsh domstate failed: {}", stderr.trim()));
        }
        let state = String::from_utf8_lossy(&out.stdout).trim().to_lowercase();
        let status = match state.as_str() {
            // `paused` from virsh == libvirt "paused" == our suspended. The
            // workspace service layer distinguishes `paused` via its own
            // session-state column; at the container runtime level we report
            // it as Running (the process is alive, just frozen).
            "running" | "paused" | "idle" => ContainerStatus::Running,
            "shut off" | "shutdown" => ContainerStatus::Stopped,
            "crashed" | "pmsuspended" | "blocked" => ContainerStatus::Unknown,
            _ => ContainerStatus::Unknown,
        };
        debug!(
            target: "node_backend::workspace",
            tenant = %container_id,
            raw_state = %state,
            status = %status.as_str(),
            "libvirt domain state queried"
        );
        Ok(status)
    }

    async fn exec_in_container(
        &self,
        container_id: &str,
        command: &[&str],
    ) -> Result<String, String> {
        let tenant_ip = {
            let tenants = self.tenants.lock().await;
            let state = tenants
                .get(container_id)
                .ok_or_else(|| format!("tenant {} not tracked", container_id))?;
            state
                .tenant_ip
                .clone()
                .ok_or_else(|| "tenant has no IP yet (did you call start_container?)".to_string())?
        };

        let mut args: Vec<String> = vec![
            "-i".to_string(),
            self.tenant_key_path.clone(),
            "-o".to_string(),
            "StrictHostKeyChecking=no".to_string(),
            "-o".to_string(),
            "UserKnownHostsFile=/dev/null".to_string(),
            "-o".to_string(),
            "ConnectTimeout=10".to_string(),
            "-o".to_string(),
            "BatchMode=yes".to_string(),
            format!("tenant@{}", tenant_ip),
        ];
        args.extend(command.iter().map(|s| s.to_string()));
        let arg_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

        let out = run_cmd_with_timeout(
            "ssh",
            &arg_refs,
            SSH_EXEC_TIMEOUT_SECS,
            "ssh exec",
        )
        .await?;
        if !out.status.success() {
            return Err(format!(
                "ssh failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).to_string())
    }

    fn runtime_name(&self) -> &'static str {
        "libvirt-qemu"
    }

    async fn get_access_url(&self, container_id: &str) -> Result<Option<String>, String> {
        let tenants = self.tenants.lock().await;
        let state = tenants
            .get(container_id)
            .ok_or_else(|| format!("tenant {} not tracked", container_id))?;
        Ok(Some(format!(
            "ssh://tenant@[{}]:{}",
            self.host_ipv6, state.host_port
        )))
    }

    /// Idempotent pause: if the domain is already in `paused` state (e.g. the
    /// KVM kernel auto-paused on host memory pressure, disk ENOSPC, or a
    /// previous pause command already succeeded but the caller retried), we
    /// return Ok without re-issuing `virsh suspend` — because libvirt returns
    /// `VIR_ERR_OPERATION_INVALID` on already-paused domains. After the
    /// suspend call we debounce the confirmation: 2 consecutive `paused`
    /// reads 200ms apart, up to a 2s overall deadline. The debounce guards
    /// against `virsh domstate` returning stale state under libvirtd lock
    /// contention (longstanding known issue — see
    /// ansible-collections/community.libvirt#63).
    async fn pause_container(&self, container_id: &str) -> Result<(), String> {
        // Fast-path idempotency: if already paused, skip the suspend call.
        if let Ok(state) = self.read_domstate(container_id).await {
            if state == "paused" {
                debug!(
                    target: "node_backend::workspace",
                    tenant = %container_id,
                    "pause_container: already paused (out-of-band or retry), no-op"
                );
                return Ok(());
            }
        }

        let out = run_cmd_with_timeout(
            "virsh",
            &["suspend", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh suspend",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // If suspend failed but the actual domstate is already paused, accept
            // as success (covers out-of-band auto-pause + error races).
            if let Ok(state) = self.read_domstate(container_id).await {
                if state == "paused" {
                    warn!(
                        target: "node_backend::workspace",
                        tenant = %container_id,
                        stderr = %stderr,
                        "virsh suspend returned error but domain is already paused — treating as success"
                    );
                    return Ok(());
                }
            }
            return Err(format!("virsh suspend failed: {}", stderr.trim()));
        }

        // Debounce confirmation: 2 consecutive paused reads 200ms apart,
        // within a 2s overall deadline.
        self.confirm_domstate_debounced(container_id, "paused", 2, 2000, 200)
            .await?;

        info!(
            target: "node_backend::workspace",
            tenant = %container_id,
            "libvirt tenant suspended"
        );
        Ok(())
    }

    /// Idempotent resume, mirror of `pause_container`. Already-running
    /// domains fast-path to Ok. systemd-timesyncd in the guest handles
    /// clock-jump recovery, so we do NOT run `virsh domtime --sync`
    /// (which requires qemu-guest-agent in the guest and the fsfreeze
    /// family is fragile under load — see libvirt-users fsfreeze-busy
    /// thread).
    async fn resume_container(&self, container_id: &str) -> Result<(), String> {
        if let Ok(state) = self.read_domstate(container_id).await {
            if state == "running" {
                debug!(
                    target: "node_backend::workspace",
                    tenant = %container_id,
                    "resume_container: already running, no-op"
                );
                return Ok(());
            }
        }

        let out = run_cmd_with_timeout(
            "virsh",
            &["resume", container_id],
            VIRSH_TIMEOUT_SECS,
            "virsh resume",
        )
        .await?;
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr);
            if let Ok(state) = self.read_domstate(container_id).await {
                if state == "running" {
                    warn!(
                        target: "node_backend::workspace",
                        tenant = %container_id,
                        stderr = %stderr,
                        "virsh resume returned error but domain is already running — treating as success"
                    );
                    return Ok(());
                }
            }
            return Err(format!("virsh resume failed: {}", stderr.trim()));
        }

        self.confirm_domstate_debounced(container_id, "running", 2, 2000, 200)
            .await?;

        info!(
            target: "node_backend::workspace",
            tenant = %container_id,
            "libvirt tenant resumed"
        );
        Ok(())
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// F3 verify: disk overlay arg sets cache=writethrough.
    /// Defense against power-loss / kernel-panic qcow2 corruption.
    #[test]
    fn disk_overlay_arg_uses_writethrough() {
        let arg = build_disk_overlay_arg("/srv/tenants/overlays/tenant-abc.qcow2");
        assert!(
            arg.contains("cache=writethrough"),
            "disk arg must include cache=writethrough; got: {}",
            arg
        );
        assert!(arg.contains("format=qcow2"), "disk arg must declare qcow2 format");
        assert!(arg.contains("bus=virtio"), "disk arg must declare virtio bus");
        assert!(
            arg.starts_with("path=/srv/tenants/overlays/tenant-abc.qcow2"),
            "path should be first argument"
        );
        // Defense against accidental switch to weaker cache mode.
        assert!(
            !arg.contains("cache=writeback"),
            "qcow2 cache must NOT be writeback (durability regression)"
        );
        assert!(
            !arg.contains("cache=unsafe"),
            "qcow2 cache must NOT be unsafe"
        );
    }

    #[test]
    fn tenant_state_is_debug_printable() {
        let s = TenantState {
            host_port: 2201,
            tenant_ip: Some("192.168.122.10".to_string()),
            socat_pid: Some(1234),
            overlay_path: "/srv/tenants/overlays/tenant-abc.qcow2".to_string(),
            seed_path: "/srv/tenants/seeds/tenant-abc-seed.iso".to_string(),
            serial_log_path: "/srv/tenants/serial-logs/tenant-abc.log".to_string(),
            vnc_ws_port: Some(5700),
        };
        let dbg = format!("{:?}", s);
        assert!(dbg.contains("2201"));
        assert!(dbg.contains("192.168.122.10"));
    }

    #[test]
    fn port_base_constants_are_in_range() {
        // Sanity: allocated ports stay in the firewall-open range 2200-2299.
        assert_eq!(V6_PORT_BASE, 2200);
        assert!(V6_PORT_RANGE <= 100);
        let max = V6_PORT_BASE + V6_PORT_RANGE - 1;
        assert!(max < 2300, "allocated port range must stay within the phase1-m1-allow-tenant-ports-v6 firewall rule (2200-2299)");
    }
}
