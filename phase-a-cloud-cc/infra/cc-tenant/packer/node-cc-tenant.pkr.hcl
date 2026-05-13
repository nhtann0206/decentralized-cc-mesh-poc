// Phase A.5 — Packer config for CC tenant base image.
//
// Bakes everything that startup-script.sh used to do at runtime:
//   - apt deps (xfce4 desktop, tigervnc, nginx, build-essential, libssl-dev)
//   - Rust toolchain + snpguest binary at /usr/local/bin/snpguest
//
// Result: a custom image `node-cc-tenant` family that boots in ~60s instead
// of ~10-15 min, while keeping SEV-SNP guest-OS features so tenants spawned
// from it can still attest. Per-tenant config (read metadata, generate
// attestation, fetch VCEK from AMD KDS, write manifest) stays in
// `startup-script.sh` since those steps depend on per-spawn nonce + chip ID.
//
// Build:
//   cd infra/gcp/cc-tenant/packer
//   packer init .
//   PROJECT=$(gcloud config get-value project) IMAGE_VERSION=$(date +%Y%m%d-%H%M) \
//     packer build -var "project_id=$PROJECT" -var "image_version=$IMAGE_VERSION" .
//
// After: `gcloud compute images list --filter=family:node-cc-tenant`

packer {
  required_plugins {
    googlecompute = {
      source  = "github.com/hashicorp/googlecompute"
      version = "~> 1.1"
    }
  }
}

variable "project_id" {
  type = string
}

variable "zone" {
  type    = string
  default = "us-central1-a"
}

variable "image_version" {
  type    = string
  default = "v1"
}

source "googlecompute" "cc-tenant" {
  project_id              = var.project_id
  zone                    = var.zone
  source_image_family     = "ubuntu-2404-lts-amd64"
  source_image_project_id = ["ubuntu-os-cloud"]
  // Bumped down from n2d-standard-4 to fit within 16-vCPU regional quota.
  // Build is single-threaded (apt + cargo install snpguest), so 2 vCPU is fine
  // — adds ~1-2 min to the bake but no functional impact.
  machine_type            = "n2d-standard-2"

  // Resulting image name + family. The deployed backend selects this
  // family via env GCP_VM_CC_IMAGE.
  image_name   = "node-cc-tenant-${var.image_version}"
  image_family = "node-cc-tenant"

  // Preserve SEV-SNP capability flag from the source Ubuntu image so
  // spawned tenants can still use --confidential-compute-type=SEV_SNP.
  // Per https://docs.cloud.google.com/compute/docs/images/configuring-imported-images
  // GCP carries guestOsFeatures across when explicitly listed.
  image_guest_os_features = [
    "SEV_CAPABLE",
    "SEV_SNP_CAPABLE",
    "UEFI_COMPATIBLE",
    "VIRTIO_SCSI_MULTIQUEUE",
    "GVNIC",
  ]

  // SSH via IAP tunnel (no need for public ingress on builder VM).
  use_iap      = true
  ssh_username = "packer"

  // Builder VM is ephemeral; deleted after image is captured.
  disk_size = 20
}

build {
  sources = ["source.googlecompute.cc-tenant"]

  // Wait for cloud-init to finish reconfiguring sources.list (race with
  // google-startup-scripts, see GCP guest-agent issue #84).
  provisioner "shell" {
    inline = ["cloud-init status --wait"]
  }

  // Bake apt packages + Rust + snpguest.
  provisioner "shell" {
    script          = "scripts/bake-cc-tools.sh"
    execute_command = "sudo -E bash -e {{.Path}}"
  }

  // Trim transient state so the image is reproducible.
  provisioner "shell" {
    inline = [
      "sudo rm -rf /tmp/* /var/tmp/* /root/.cache /home/*/.cache",
      "sudo journalctl --vacuum-time=1d",
      "sudo apt-get clean",
    ]
    execute_command = "bash -e {{.Path}}"
  }
}
