use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::process::Command;
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use crate::services::terminal::pty_manager::PtyManager;

// ============================================================================
// Container Configuration
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContainerConfig {
    pub image: String,
    pub name: Option<String>,
    pub ram_mb: u32,
    pub cpu_cores: f32,
    pub environment: HashMap<String, String>,
    pub working_dir: Option<String>,
    pub command: Option<Vec<String>>,
    /// Confidential Compute settings. When `Some`, the runtime will request
    /// hardware-backed isolation (currently only `GcpComputeProvider` honors
    /// this — Docker/Pty/libvirt runtimes ignore the field). The runtime is
    /// responsible for translating these settings into provider-specific
    /// launch parameters (e.g. SEV-SNP for GCP N2D).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cc: Option<CcConfig>,
}

/// Confidential Compute configuration for a workspace tenant.
///
/// Only `GcpComputeProvider` currently honors this — Docker, Pty, and libvirt
/// runtimes will ignore the field. Other CC backends (OP-TEE PTA on Radxa,
/// future Keystone) plug in by adding new variants here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum CcConfig {
    /// AMD SEV-SNP via GCP N2D Confidential VM.
    /// `nonce_hex` is 64 hex chars (32 bytes) bound into the attestation
    /// report's `report_data` field for replay protection. Bob (the marketplace
    /// operator) generates this and verifies it after the tenant boots.
    SevSnp { nonce_hex: String },
}

// ============================================================================
// Container Status
// ============================================================================

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContainerStatus {
    Created,
    Running,
    Stopped,
    Removed,
    Unknown,
}

impl ContainerStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ContainerStatus::Created => "created",
            ContainerStatus::Running => "running",
            ContainerStatus::Stopped => "stopped",
            ContainerStatus::Removed => "removed",
            ContainerStatus::Unknown => "unknown",
        }
    }

    pub fn from_docker_status(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "created" => ContainerStatus::Created,
            "running" => ContainerStatus::Running,
            "exited" | "stopped" | "dead" => ContainerStatus::Stopped,
            "removing" => ContainerStatus::Removed,
            _ => ContainerStatus::Unknown,
        }
    }
}

// ============================================================================
// Container Runtime Trait
// ============================================================================

#[async_trait]
pub trait ContainerRuntime: Send + Sync {
    /// Create a new container from the given configuration.
    /// Returns the container ID on success.
    async fn create_container(&self, config: ContainerConfig) -> Result<String, String>;

    /// Start a previously created container.
    async fn start_container(&self, container_id: &str) -> Result<(), String>;

    /// Stop a running container (graceful with timeout).
    async fn stop_container(&self, container_id: &str) -> Result<(), String>;

    /// Remove a container (force).
    async fn remove_container(&self, container_id: &str) -> Result<(), String>;

    /// Query the current status of a container.
    async fn get_container_status(&self, container_id: &str) -> Result<ContainerStatus, String>;

    /// Execute a command inside a running container.
    /// Returns the combined stdout output.
    async fn exec_in_container(
        &self,
        container_id: &str,
        command: &[&str],
    ) -> Result<String, String>;

    /// Return the name of this runtime implementation.
    fn runtime_name(&self) -> &'static str;

    /// Get a browser-accessible URL for this container/VM.
    /// Returns None for terminal-based runtimes (Docker, PTY).
    /// GCP provider overrides to return the Guacamole/XRDP access URL.
    async fn get_access_url(&self, _container_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }

    /// Pause a running container/VM. For libvirt-backed runtimes this is
    /// `virsh suspend` — RAM-resident freeze, ~25ms on c2-standard-8 per M1 bench.
    /// Default impl fails because pause is meaningful only for runtimes that
    /// keep in-memory state (currently libvirt); Docker/PTY sessions are
    /// restarted, not paused, by the billing loop.
    async fn pause_container(&self, _container_id: &str) -> Result<(), String> {
        Err(format!("pause not supported by runtime {}", self.runtime_name()))
    }

    /// Resume a paused container/VM (`virsh resume` for libvirt). Paired with
    /// `pause_container` above.
    async fn resume_container(&self, _container_id: &str) -> Result<(), String> {
        Err(format!("resume not supported by runtime {}", self.runtime_name()))
    }

    /// Downcast to concrete type for provider-specific operations (e.g., snapshots).
    fn as_any(&self) -> &dyn std::any::Any;
}

// ============================================================================
// Docker Runtime
// ============================================================================

pub struct DockerRuntime {
    docker_bin: String,
}

impl DockerRuntime {
    /// Attempt to create a DockerRuntime by verifying Docker is available.
    /// Returns `None` if the `docker` CLI is not reachable.
    pub async fn new() -> Option<Self> {
        let output = Command::new("docker")
            .args(["version", "--format", "{{.Server.Version}}"])
            .output()
            .await;

        match output {
            Ok(out) if out.status.success() => {
                let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
                info!(
                    target: "node_backend::workspace",
                    docker_version = %version,
                    "Docker runtime detected"
                );
                Some(Self {
                    docker_bin: "docker".to_string(),
                })
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                debug!(
                    target: "node_backend::workspace",
                    stderr = %stderr,
                    "Docker CLI returned non-zero exit code"
                );
                None
            }
            Err(e) => {
                debug!(
                    target: "node_backend::workspace",
                    error = %e,
                    "Docker CLI not available"
                );
                None
            }
        }
    }
}

#[async_trait]
impl ContainerRuntime for DockerRuntime {
    async fn create_container(&self, config: ContainerConfig) -> Result<String, String> {
        let mut args = vec!["create".to_string()];

        if let Some(ref name) = config.name {
            args.push("--name".to_string());
            args.push(name.clone());
        }

        args.push("--memory".to_string());
        args.push(format!("{}m", config.ram_mb));
        args.push("--cpus".to_string());
        args.push(format!("{}", config.cpu_cores));

        for (key, val) in &config.environment {
            args.push("-e".to_string());
            args.push(format!("{}={}", key, val));
        }

        if let Some(ref workdir) = config.working_dir {
            args.push("-w".to_string());
            args.push(workdir.clone());
        }

        args.push(config.image.clone());

        if let Some(ref cmd) = config.command {
            args.extend(cmd.clone());
        }

        let output = Command::new(&self.docker_bin)
            .args(&args)
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker create: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                target: "node_backend::workspace",
                image = %config.image,
                stderr = %stderr,
                "docker create failed"
            );
            return Err(format!("docker create failed: {}", stderr.trim()));
        }

        let container_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            image = %config.image,
            ram_mb = config.ram_mb,
            cpu_cores = config.cpu_cores,
            "Container created"
        );

        Ok(container_id)
    }

    async fn start_container(&self, container_id: &str) -> Result<(), String> {
        let output = Command::new(&self.docker_bin)
            .args(["start", container_id])
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker start: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                target: "node_backend::workspace",
                container_id = %container_id,
                stderr = %stderr,
                "docker start failed"
            );
            return Err(format!("docker start failed: {}", stderr.trim()));
        }

        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            "Container started"
        );
        Ok(())
    }

    async fn stop_container(&self, container_id: &str) -> Result<(), String> {
        let output = Command::new(&self.docker_bin)
            .args(["stop", "-t", "5", container_id])
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker stop: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                target: "node_backend::workspace",
                container_id = %container_id,
                stderr = %stderr,
                "docker stop failed"
            );
            return Err(format!("docker stop failed: {}", stderr.trim()));
        }

        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            "Container stopped"
        );
        Ok(())
    }

    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        let output = Command::new(&self.docker_bin)
            .args(["rm", "-f", container_id])
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker rm: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                target: "node_backend::workspace",
                container_id = %container_id,
                stderr = %stderr,
                "docker rm failed"
            );
            return Err(format!("docker rm failed: {}", stderr.trim()));
        }

        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            "Container removed"
        );
        Ok(())
    }

    async fn get_container_status(&self, container_id: &str) -> Result<ContainerStatus, String> {
        let output = Command::new(&self.docker_bin)
            .args([
                "inspect",
                "--format",
                "{{.State.Status}}",
                container_id,
            ])
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker inspect: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("No such") {
                return Ok(ContainerStatus::Removed);
            }
            return Err(format!("docker inspect failed: {}", stderr.trim()));
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let status = ContainerStatus::from_docker_status(&raw);

        debug!(
            target: "node_backend::workspace",
            container_id = %container_id,
            raw_status = %raw,
            status = %status.as_str(),
            "Container status queried"
        );

        Ok(status)
    }

    async fn exec_in_container(
        &self,
        container_id: &str,
        command: &[&str],
    ) -> Result<String, String> {
        let mut args = vec!["exec", container_id];
        args.extend(command);

        let output = Command::new(&self.docker_bin)
            .args(&args)
            .output()
            .await
            .map_err(|e| format!("Failed to execute docker exec: {}", e))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            error!(
                target: "node_backend::workspace",
                container_id = %container_id,
                stderr = %stderr,
                "docker exec failed"
            );
            return Err(format!("docker exec failed: {}", stderr.trim()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        Ok(stdout)
    }

    fn runtime_name(&self) -> &'static str {
        "docker"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ============================================================================
// PTY Fallback Runtime
// ============================================================================

/// Fallback runtime that wraps the existing PtyManager when no container engine
/// (Docker/Podman) is available. Provides a minimal container-like interface on
/// top of local PTY sessions.
pub struct PtyFallbackRuntime {
    pty_manager: Arc<PtyManager>,
}

impl PtyFallbackRuntime {
    pub fn new(pty_manager: Arc<PtyManager>) -> Self {
        info!(
            target: "node_backend::workspace",
            "PTY fallback runtime initialized (no container engine available)"
        );
        Self { pty_manager }
    }
}

#[async_trait]
impl ContainerRuntime for PtyFallbackRuntime {
    async fn create_container(&self, config: ContainerConfig) -> Result<String, String> {
        let container_id = Uuid::new_v4().to_string();
        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            image = %config.image,
            "PTY fallback: pseudo-container created (no isolation)"
        );
        Ok(container_id)
    }

    async fn start_container(&self, container_id: &str) -> Result<(), String> {
        let pid = self
            .pty_manager
            .spawn_pty(
                container_id.to_string(),
                "workspace".to_string(),
                Some("/bin/bash".to_string()),
                80,
                24,
            )
            .await?;

        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            pid = pid,
            "PTY fallback: container started via PTY"
        );
        Ok(())
    }

    async fn stop_container(&self, container_id: &str) -> Result<(), String> {
        if self.pty_manager.has_session(container_id).await {
            self.pty_manager.terminate_pty(container_id).await?;
            info!(
                target: "node_backend::workspace",
                container_id = %container_id,
                "PTY fallback: container stopped (PTY terminated)"
            );
        } else {
            debug!(
                target: "node_backend::workspace",
                container_id = %container_id,
                "PTY fallback: stop requested but no PTY session found"
            );
        }
        Ok(())
    }

    async fn remove_container(&self, container_id: &str) -> Result<(), String> {
        if self.pty_manager.has_session(container_id).await {
            self.pty_manager.terminate_pty(container_id).await?;
        }
        info!(
            target: "node_backend::workspace",
            container_id = %container_id,
            "PTY fallback: container removed"
        );
        Ok(())
    }

    async fn get_container_status(&self, container_id: &str) -> Result<ContainerStatus, String> {
        if self.pty_manager.has_session(container_id).await {
            Ok(ContainerStatus::Running)
        } else {
            Ok(ContainerStatus::Stopped)
        }
    }

    async fn exec_in_container(
        &self,
        container_id: &str,
        command: &[&str],
    ) -> Result<String, String> {
        if !self.pty_manager.has_session(container_id).await {
            return Err(format!(
                "PTY fallback: no active session for container {}",
                container_id
            ));
        }

        let cmd_line = command.join(" ");
        self.pty_manager
            .write_input(container_id, &format!("{}\n", cmd_line))
            .await?;

        warn!(
            target: "node_backend::workspace",
            container_id = %container_id,
            command = %cmd_line,
            "PTY fallback: exec piped to PTY stdin (output captured via WebSocket, not returned here)"
        );

        Ok(String::new())
    }

    fn runtime_name(&self) -> &'static str {
        "pty-fallback"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

// ============================================================================
// Runtime Detection Factory
// ============================================================================

/// Detect the best available container runtime.
///
/// Priority: explicit `WORKSPACE_RUNTIME=libvirt` (Phase 1 local hypervisor)
/// → GCP Compute (when `GCP_PROJECT_ID` set) → Docker → PTY fallback → Noop.
pub async fn detect_runtime(pty_manager: Option<Arc<PtyManager>>) -> Arc<dyn ContainerRuntime> {
    // Explicit libvirt opt-in for Phase 1 nested-KVM demo.
    if std::env::var("WORKSPACE_RUNTIME")
        .map(|v| v.eq_ignore_ascii_case("libvirt"))
        .unwrap_or(false)
    {
        match super::libvirt_provider::LibvirtQemuRuntime::from_env().await {
            Ok(provider) => {
                info!(
                    target: "node_backend::workspace",
                    "Container runtime: libvirt + QEMU (nested KVM)"
                );
                return Arc::new(provider);
            }
            Err(e) => {
                warn!(
                    target: "node_backend::workspace",
                    error = %e,
                    "Libvirt runtime init failed despite WORKSPACE_RUNTIME=libvirt, falling through"
                );
            }
        }
    }

    // GCP Compute Engine — highest priority when configured
    if std::env::var("GCP_PROJECT_ID").is_ok() {
        match super::gcp_provider::GcpComputeProvider::from_env().await {
            Ok(provider) => {
                info!(
                    target: "node_backend::workspace",
                    "Container runtime: GCP Compute"
                );
                return Arc::new(provider);
            }
            Err(e) => {
                warn!(
                    target: "node_backend::workspace",
                    error = %e,
                    "GCP Compute init failed, falling back to Docker"
                );
            }
        }
    }

    if let Some(docker) = DockerRuntime::new().await {
        info!(
            target: "node_backend::workspace",
            "Container runtime: Docker"
        );
        return Arc::new(docker);
    }

    if let Some(pty) = pty_manager {
        info!(
            target: "node_backend::workspace",
            "Container runtime: PTY fallback (Docker not available)"
        );
        return Arc::new(PtyFallbackRuntime::new(pty));
    }

    warn!(
        target: "node_backend::workspace",
        "No container runtime available (Docker absent, PtyManager not provided)"
    );
    Arc::new(NoopRuntime)
}

// ============================================================================
// No-Op Runtime (safety net)
// ============================================================================

struct NoopRuntime;

#[async_trait]
impl ContainerRuntime for NoopRuntime {
    async fn create_container(&self, _config: ContainerConfig) -> Result<String, String> {
        Err("No container runtime available".to_string())
    }

    async fn start_container(&self, _container_id: &str) -> Result<(), String> {
        Err("No container runtime available".to_string())
    }

    async fn stop_container(&self, _container_id: &str) -> Result<(), String> {
        Err("No container runtime available".to_string())
    }

    async fn remove_container(&self, _container_id: &str) -> Result<(), String> {
        Err("No container runtime available".to_string())
    }

    async fn get_container_status(&self, _container_id: &str) -> Result<ContainerStatus, String> {
        Err("No container runtime available".to_string())
    }

    async fn exec_in_container(
        &self,
        _container_id: &str,
        _command: &[&str],
    ) -> Result<String, String> {
        Err("No container runtime available".to_string())
    }

    fn runtime_name(&self) -> &'static str {
        "none"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
