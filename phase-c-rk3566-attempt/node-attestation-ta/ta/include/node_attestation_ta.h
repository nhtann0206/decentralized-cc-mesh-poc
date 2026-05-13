/* SPDX-License-Identifier: BSD-2-Clause */
/*
 * NodeOS attestation Trusted Application — public header.
 *
 * Shared between the TA itself and any host-side client (Rust libteec
 * binding in `packages/backend/src/tee/optee_provider.rs`). Contains the
 * TA UUID and the command IDs the client invokes.
 *
 * Phase C — runs inside OP-TEE Secure World on Radxa Zero 3W (RK3566).
 * Real hardware-rooted CC: persistent RSA keypair derived from the chip's
 * HUK via OP-TEE's TSK (TA Storage Key) chain. See README.md for the
 * cryptographic chain rationale.
 */

#ifndef NODE_ATTESTATION_TA_H
#define NODE_ATTESTATION_TA_H

/*
 * UUID of this TA — fresh-generated 2026-05-03. Keep stable; clients pin
 * attestation evidence against (peer node id, this UUID).
 */
#define NODE_ATTESTATION_UUID \
	{ 0xfeb95976, 0x43e8, 0x4e43, \
		{ 0xa7, 0x9c, 0x51, 0x03, 0x59, 0xff, 0xf6, 0x33 } }

/*
 * CMD_GET_PUBKEY — retrieve the TA's persistent RSA-2048 public key.
 *
 *   in/out params[0].memref  modulus   (256 bytes when key is 2048-bit)
 *   in/out params[1].memref  exponent  (typically 3 bytes for 0x010001)
 *
 * On first invocation the keypair is generated and persisted to OP-TEE
 * secure storage (encrypted with TSK derived from chip HUK). Subsequent
 * invocations return the same key.
 *
 * The relying party (Alice) pins (modulus, exponent) on first contact
 * (TOFU) and rejects later evidence that doesn't match.
 */
#define NODE_ATTESTATION_CMD_GET_PUBKEY 0x1000

/*
 * CMD_ATTEST — sign a fresh challenge with the persistent key.
 *
 *   in  params[0].memref  nonce         (exactly 32 bytes; relying-party-supplied)
 *   out params[1].memref  measurement   (32 bytes — SHA-256 of TA build marker)
 *   out params[2].memref  signature     (256 bytes — RSASSA-PSS-SHA256 of
 *                                        SHA-256(nonce || measurement))
 *
 * Algorithm: TEE_ALG_RSASSA_PKCS1_PSS_MGF1_SHA256, salt length matches
 * digest length (32 bytes). Same family as upstream PTA attestation, so
 * Alice's verifier doesn't care whether the signer is a PTA or a TA.
 */
#define NODE_ATTESTATION_CMD_ATTEST 0x1001

/*
 * Sizes the host code can rely on (Phase C minimum: hardcoded for the
 * 2048-bit / SHA-256 build).
 */
#define NODE_ATTESTATION_NONCE_SIZE 32
#define NODE_ATTESTATION_MEASUREMENT_SIZE 32
#define NODE_ATTESTATION_SIGNATURE_SIZE 256
#define NODE_ATTESTATION_MODULUS_SIZE 256
#define NODE_ATTESTATION_KEY_BITS 2048

#endif /* NODE_ATTESTATION_TA_H */
