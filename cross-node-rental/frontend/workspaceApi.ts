import { apiClient } from './apiClient';

// ─── Workspace Types ────────────────────────────────────────────

export type WorkspaceStep =
  | 'browse'
  | 'verify'
  | 'configure'
  | 'confirm'
  | 'running'
  | 'terminal';

export type MachineStatus = 'available' | 'busy' | 'offline';
export type VerificationStatus = 'pending' | 'verifying' | 'passed' | 'failed' | 'skipped';
export type SessionStatus = 'starting' | 'running' | 'pausing' | 'paused' | 'resuming' | 'stopping' | 'stopped' | 'error';

export interface SecureComputer {
  id: string;
  node_id: string;
  location: string;
  machine_type: string;
  price_sats_per_min: number;
  status: MachineStatus;
  ram_gb: number;
  storage_gb: number;
  uptime_hours: number;
  attestation_valid: boolean;
}

export interface VerificationStep {
  id: string;
  label: string;
  description: string;
  status: VerificationStatus;
  /** Whether this check was backed by real hardware (ATECC608A + TPM). */
  is_hardware_backed: boolean;
}

export interface WorkspaceConfig {
  computer_id: string;
  ram_gb: number;
  storage_gb: number;
  environment: string;
}

export interface WorkspaceSession {
  id: string;
  computer_id: string;
  computer_location: string;
  status: SessionStatus;
  started_at: string;
  elapsed_seconds: number;
  cost_sats: number;
  price_sats_per_min: number;
  runtime_type: string;
  container_id: string | null;
  access_url: string | null;
  /** M4: BOLT12 offer for per-minute billing (null = credit billing fallback) */
  bolt12_offer?: string | null;
}

export interface StartSessionRequest {
  computer_id: string;
  config: WorkspaceConfig;
}

export interface StartSessionResponse {
  session_id: string;
  status: SessionStatus;
}

export interface StopSessionResponse {
  status: string;
  cost_sats: number;
  elapsed_seconds: number;
  balance_sats: number;
}

export interface BalanceResponse {
  balance_sats: number;
}

// ─── Backup Types ───────────────────────────────────────────────

export interface BackupInfo {
  name: string;
  status: string;
  created_at: string;
  disk_size_gb: number;
  storage_bytes: number;
}

export interface CreateBackupResponse {
  snapshot_name: string;
  status: string;
}

export interface RestoreBackupResponse {
  new_session_id: string;
  status: string;
}

/**
 * Confidential Compute attestation evidence verified by Bob's backend.
 *
 * Mirrors `CcAttestationDto` (tagged enum) in
 * `packages/backend/src/workspace/handlers.rs`. The wire shape is a flat
 * object discriminated by `provider`:
 *   - `amd-sev-snp` (Phase A) — GCP N2D Confidential VMs, AMD VCEK chain
 *   - `optee-attestation` (Phase C) — Radxa Zero 3W RK3566, OP-TEE PTA
 *
 * Use the `provider` discriminator to narrow before reading variant-specific
 * fields. `CcAttestationPanel` switches on this in the rent wizard step 2/3.
 */
export type CcAttestation = SevSnpAttestation | OpteeAttestation;

/**
 * AMD SEV-SNP variant — Phase A. Hex fields match `snpguest display report`
 * so operators can cross-check with AMD tooling.
 */
export interface SevSnpAttestation {
  provider: 'amd-sev-snp';
  /** SHA-384 launch measurement (96 hex chars = 48 bytes). */
  measurement_hex: string;
  /** AMD chip ID, per-physical-CPU unique (128 hex chars = 64 bytes). */
  chip_id_hex: string;
  /** 64-byte report_data field — Bob's nonce echoed by the chip. */
  report_data_hex: string;
  /** VMPL the guest ran at when generating the report (0..=3). */
  vmpl: number;
  /** Guest policy bitfield as hex (e.g. "0x0000000000030000"). */
  policy_hex: string;
  /** Current TCB component versions (microcode/SNP/PSP/boot loader/FMC). */
  current_tcb: {
    bootloader: number;
    tee: number;
    snp: number;
    microcode: number;
    fmc?: number;
  };
}

/**
 * OP-TEE attestation PTA variant — Phase C. The RK3566 chip's HUK derives
 * an RSA-2048 keypair; the PTA signs `(nonce || measurement)` with
 * RSA-PSS-SHA256. Alice verifies by reconstructing the digest and running
 * `openssl rsa-pss verify` against the supplied modulus + exponent.
 */
export interface OpteeAttestation {
  provider: 'optee-attestation';
  /** SHA-256 measurement (32 bytes hex) over BL31 || BL32 || kernel || cmdline. */
  measurement_hex: string;
  /** Bob-supplied nonce binding this report to a session (32 bytes hex). */
  nonce_hex: string;
  /** RSA public key modulus (256 bytes hex for RSA-2048). */
  pubkey_modulus_hex: string;
  /** RSA public key exponent — typically `010001` (= 65537). */
  pubkey_exponent_hex: string;
  /** RSA-PSS-SHA256 signature (256 bytes hex for RSA-2048). */
  signature_hex: string;
  /** UUID of the TA that produced the quote (16 bytes hex). */
  ta_uuid_hex: string;
  /** BL31 (TF-A) version string. */
  bl31_version: string;
  /** BL32 (OP-TEE OS) version string. */
  bl32_version: string;
}

// ─── API Functions ──────────────────────────────────────────────

export const workspaceApi = {
  getBalance: () =>
    apiClient.get<BalanceResponse>('/v2/workspace/balance'),

  listComputers: () =>
    apiClient.get<SecureComputer[]>('/v2/workspace/computers'),

  verifyComputer: (computerId: string) =>
    apiClient.post<VerificationStep[]>(`/v2/workspace/computers/${computerId}/verify`, {}),

  startSession: (request: StartSessionRequest) =>
    apiClient.post<StartSessionResponse>('/v2/workspace/sessions', request),

  getSession: (sessionId: string) =>
    apiClient.get<WorkspaceSession>(`/v2/workspace/sessions/${sessionId}`),

  stopSession: (sessionId: string) =>
    apiClient.post<StopSessionResponse>(`/v2/workspace/sessions/${sessionId}/stop`, {}),

  createBackup: (sessionId: string) =>
    apiClient.post<CreateBackupResponse>(`/v2/workspace/sessions/${sessionId}/backup`, {}),

  listBackups: (sessionId: string) =>
    apiClient.get<BackupInfo[]>(`/v2/workspace/sessions/${sessionId}/backups`),

  restoreBackup: (sessionId: string, snapshotName: string) =>
    apiClient.post<RestoreBackupResponse>(`/v2/workspace/sessions/${sessionId}/restore`, {
      snapshot_name: snapshotName,
    }),
};

// ─── M4: Peer Workspace API (cross-node VM rental) ─────────────
// Alice calls these to rent VMs from a peer (Bob). Requests are
// proxied through Alice's backend → L402HttpClient → Bob's external
// workspace handlers. The `nodeId` identifies the target peer.

export const peerWorkspaceApi = {
  listComputers: (nodeId: string) =>
    apiClient.get<SecureComputer[]>(`/workspace/peer/${nodeId}/computers`),

  startSession: (nodeId: string, request: StartSessionRequest) =>
    apiClient.post<StartSessionResponse>(`/workspace/peer/${nodeId}/sessions`, request),

  getSession: (nodeId: string, sessionId: string) =>
    apiClient.get<WorkspaceSession>(`/workspace/peer/${nodeId}/sessions/${sessionId}`),

  stopSession: (nodeId: string, sessionId: string) =>
    apiClient.post<StopSessionResponse>(`/workspace/peer/${nodeId}/sessions/${sessionId}/stop`, {}),

  // M7: aggregate view of all rentals across peers. Reads the local cache
  // populated by peer_start_session and refreshed by a cron task on the
  // user's own backend — so this call never hits a peer directly.
  listMyRentals: () => apiClient.get<MyRentalView[]>(`/workspace/my-rentals`),

  // Phase A — pre-rent CC proof. Alice asks her backend, which proxies to
  // Bob's `/external/workspace/cc-attestation` over L402. Bob returns its
  // OWN host attestation; Alice's backend verifies the AMD chain offline
  // before returning. A successful response means Bob proved the platform
  // is genuine SEV-SNP — the green badge in step 2 of the rent wizard.
  getProviderCcAttestation: (nodeId: string) =>
    apiClient.get<CcAttestation>(`/workspace/peer/${nodeId}/cc-attestation`),
};

// ─── Hosting (provider side) — what peers are renting on THIS node ───

export interface HostedSession {
  session_id: string;
  renter_node_id: string;
  computer_id: string;
  computer_location: string;
  status: string;
  price_sats_per_min: number;
  cost_sats: number;
  elapsed_seconds: number;
  runtime_type: string;
  bolt12_offer?: string | null;
  started_at: string;
}

export interface HostingSummary {
  active_sessions: number;
  distinct_tenants: number;
  lifetime_revenue_sats: number;
  revenue_last_24h_sats: number;
}

export const hostingApi = {
  listHostedSessions: () =>
    apiClient.get<HostedSession[]>(`/workspace/hosting/sessions`),
  summary: () => apiClient.get<HostingSummary>(`/workspace/hosting/summary`),
  listOfferings: () =>
    apiClient.get<SecureComputer[]>(`/workspace/hosting/offerings`),
};

/// Mirrors backend `MyRentalView`. Session fields match `WorkspaceSession`
/// naming so the same card renders both shapes.
export interface MyRentalView {
  session_id: string;
  peer_node_id: string;
  computer_id: string;
  price_sats_per_min: number;
  status: string;
  cost_sats: number;
  elapsed_seconds: number;
  bolt12_offer?: string | null;
  started_at: string;
  last_poll_at?: string | null;
}
