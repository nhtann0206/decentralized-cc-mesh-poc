//! AMD SEV-SNP attestation report verifier.
//!
//! Phase A — Bob's backend uses this to independently verify the attestation
//! report fetched from a tenant N2D Confidential VM. The full chain is:
//!
//!   AMD ARK (root) → AMD ASK (intermediate) → VCEK (per-chip) → report
//!
//! Steps:
//!   1. Parse the 1184-byte report into a structured `AttestationReport`.
//!   2. Verify ARK is self-signed (matches AMD's published root key).
//!   3. Verify ASK is signed by ARK.
//!   4. Verify VCEK is signed by ASK.
//!   5. Verify the report's signature was produced by the VCEK private key
//!      (ECDSA P-384 over SHA-384 of the report's signed prefix).
//!   6. Check the `report_data` field equals the nonce Bob supplied at spawn.
//!
//! Real implementation lives behind `cfg(target_arch = "x86_64")` because the
//! `sev` crate's transitive `rdrand` dependency is x86-only. macOS arm64 dev
//! builds get a stub that returns `VerifyError::Unsupported`. Production
//! target (GCP N2D Linux/amd64) always exercises the real path.

use serde::{Deserialize, Serialize};

/// Result of a fully verified attestation. Contains the security-relevant
/// fields the caller should display, log, or compare against expected values.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct VerifiedAttestation {
    /// 48-byte SHA-384 measurement of the launch state (firmware + kernel).
    pub measurement: Vec<u8>,
    /// 64-byte AMD chip ID (per-physical-CPU unique identifier).
    pub chip_id: Vec<u8>,
    /// 64-byte report_data (nonce supplied by the verifier at quote time).
    pub report_data: Vec<u8>,
    /// VMPL the guest was running at when the report was generated.
    pub vmpl: u32,
    /// Guest policy bitfield (debug allowed, migration agent, single socket, …).
    pub policy: u64,
    /// Current TCB component versions. Sourced from `sev::firmware::host::TcbVersion`.
    pub current_tcb: TcbDto,
}

/// Flat DTO mirror of `sev::firmware::host::TcbVersion`. Kept here so callers
/// outside the verifier (HTTP handlers, frontend) don't pull in the `sev`
/// crate, which is x86_64-only and breaks Mac dev builds.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TcbDto {
    pub bootloader: u8,
    pub tee: u8,
    pub snp: u8,
    pub microcode: u8,
    pub fmc: Option<u8>,
}

#[derive(Debug, thiserror::Error)]
pub enum VerifyError {
    #[error("SEV-SNP verifier not built for this target architecture (require x86_64)")]
    Unsupported,

    #[error("attestation report bytes are wrong length: expected 1184, got {0}")]
    BadReportLength(usize),

    #[error("report parse failed: {0}")]
    ReportParse(String),

    #[error("certificate parse failed: {0}")]
    CertParse(String),

    #[error("certificate chain verification failed: {0}")]
    ChainInvalid(String),

    #[error("report signature verification failed: {0}")]
    SignatureInvalid(String),

    #[error("report_data mismatch: expected nonce binding does not match report")]
    NonceMismatch,
}

/// Verify an AMD SEV-SNP attestation report end-to-end.
///
/// Inputs:
///   - `report_bytes`: 1184-byte attestation report blob (raw output of
///     `snpguest report`, or the `/dev/sev-guest` ioctl response).
///   - `vcek_der`: VCEK certificate fetched from AMD KDS, DER-encoded.
///   - `ask_der`: AMD SEV intermediate certificate, DER-encoded.
///   - `ark_der`: AMD root certificate, DER-encoded.
///   - `expected_report_data`: the 64-byte nonce the caller supplied to the
///     guest at quote-generation time. Used for replay protection.
///
/// On success returns the verified attestation evidence. On any verification
/// failure (chain invalid, signature wrong, nonce mismatch) returns a typed
/// `VerifyError`.
#[cfg(target_arch = "x86_64")]
pub fn verify_sev_snp_attestation(
    report_bytes: &[u8],
    vcek_der: &[u8],
    ask_der: &[u8],
    ark_der: &[u8],
    expected_report_data: &[u8; 64],
) -> Result<VerifiedAttestation, VerifyError> {
    use sev::certs::snp::{ca::Chain as CaChain, Certificate, Chain, Verifiable};
    use sev::firmware::guest::AttestationReport;
    use sev::parser::ByteParser;

    if report_bytes.len() != 1184 {
        return Err(VerifyError::BadReportLength(report_bytes.len()));
    }

    // 1. Parse the report.
    let report = AttestationReport::from_bytes(report_bytes)
        .map_err(|e| VerifyError::ReportParse(format!("{e:?}")))?;

    // 2-3-4. Verify cert chain ARK → ASK → VCEK.
    let ark = Certificate::from_der(ark_der)
        .map_err(|e| VerifyError::CertParse(format!("ARK: {e:?}")))?;
    let ask = Certificate::from_der(ask_der)
        .map_err(|e| VerifyError::CertParse(format!("ASK: {e:?}")))?;
    let vcek = Certificate::from_der(vcek_der)
        .map_err(|e| VerifyError::CertParse(format!("VCEK: {e:?}")))?;

    let ca = CaChain { ark, ask };
    let chain = Chain { ca, vek: vcek };

    chain
        .verify()
        .map_err(|e| VerifyError::ChainInvalid(format!("{e:?}")))?;

    // 5. Verify the report's ECDSA signature against the VCEK public key.
    (&chain, &report)
        .verify()
        .map_err(|e| VerifyError::SignatureInvalid(format!("{e:?}")))?;

    // 6. Bind to expected nonce.
    if report.report_data != *expected_report_data {
        return Err(VerifyError::NonceMismatch);
    }

    Ok(VerifiedAttestation {
        measurement: report.measurement.to_vec(),
        chip_id: report.chip_id.to_vec(),
        report_data: report.report_data.to_vec(),
        vmpl: report.vmpl,
        policy: report.policy.0,
        current_tcb: TcbDto {
            bootloader: report.current_tcb.bootloader,
            tee: report.current_tcb.tee,
            snp: report.current_tcb.snp,
            microcode: report.current_tcb.microcode,
            fmc: report.current_tcb.fmc,
        },
    })
}

#[cfg(not(target_arch = "x86_64"))]
pub fn verify_sev_snp_attestation(
    _report_bytes: &[u8],
    _vcek_der: &[u8],
    _ask_der: &[u8],
    _ark_der: &[u8],
    _expected_report_data: &[u8; 64],
) -> Result<VerifiedAttestation, VerifyError> {
    Err(VerifyError::Unsupported)
}
