// SPDX-License-Identifier: BSD-2-Clause
/*
 * NodeOS attestation Trusted Application.
 *
 * Runs in OP-TEE Secure World on Radxa Zero 3W (Rockchip RK3566). The TA
 * owns a persistent RSA-2048 keypair that's encrypted at rest with the
 * TA Storage Key (TSK), itself derived from the chip's Hardware Unique
 * Key. Two commands:
 *
 *   - GET_PUBKEY: returns the public modulus + exponent so a remote
 *     verifier can pin them on first contact (TOFU).
 *   - ATTEST: signs `SHA-256(nonce || measurement)` with RSASSA-PSS-SHA256
 *     and returns the signature plus the measurement.
 *
 * Cryptographic chain rooting attestation in hardware:
 *
 *     chip HUK            (one-time-programmed in OTP fuses)
 *       └── SSK = HMAC(HUK, "secure-storage")
 *             └── TSK = HMAC(SSK, TA_UUID)         [per-chip + per-TA]
 *                   └── encrypts persistent objects in REE-FS
 *                         └── stores RSA-2048 keypair generated on
 *                             first invocation; survives reboots.
 *
 * Same algorithm family (RSASSA-PSS-SHA256) as the upstream PTA
 * attestation, so the relying-party verifier can dispatch on the
 * `provider = "optee-attestation"` discriminator without caring whether
 * the signer is a PTA or a regular TA.
 */

#include <inttypes.h>
#include <string.h>
#include <tee_internal_api.h>
#include <tee_internal_api_extensions.h>

#include <node_attestation_ta.h>

/*
 * Persistent object name. Versioned so we can rotate the keypair if we
 * ever change the algorithm or key size.
 */
static const char NODE_ATT_KEY_OBJ_ID[] = "node-attestation-keypair-v1";

/*
 * Build-tied measurement marker. Hashing a fixed string ties the
 * attestation signature to a specific TA build/version. Future revs
 * should replace this with a real chain measurement (BL31/BL32/kernel
 * hashes), but for Phase C minimum this is enough to prove "the
 * signature came from a TA built from this source tree".
 *
 * To rev: bump the marker AND the persistent-object name above so old
 * pinned pubkeys are invalidated cleanly.
 */
static const char TA_BUILD_MARKER[] = "node-attestation-ta@v1.0";

/*
 * Open the persistent keypair, generating + persisting it on first run.
 * Returned handle is the caller's responsibility to close.
 */
static TEE_Result open_or_create_keypair(TEE_ObjectHandle *out)
{
	TEE_Result res = TEE_ERROR_GENERIC;
	TEE_ObjectHandle pers = TEE_HANDLE_NULL;
	TEE_ObjectHandle transient = TEE_HANDLE_NULL;

	res = TEE_OpenPersistentObject(
		TEE_STORAGE_PRIVATE,
		NODE_ATT_KEY_OBJ_ID, sizeof(NODE_ATT_KEY_OBJ_ID) - 1,
		TEE_DATA_FLAG_ACCESS_READ | TEE_DATA_FLAG_ACCESS_WRITE,
		&pers);
	if (res == TEE_SUCCESS) {
		*out = pers;
		return TEE_SUCCESS;
	}
	if (res != TEE_ERROR_ITEM_NOT_FOUND) {
		EMSG("TEE_OpenPersistentObject failed: %#" PRIx32, res);
		return res;
	}

	/*
	 * First invocation on this device: generate keypair + persist.
	 * RSA-2048 keygen on Cortex-A55 takes ~30s, hence DMSG so the
	 * operator sees what's happening on the OP-TEE console.
	 */
	DMSG("Generating RSA-%d keypair (one-time, ~30s)",
	     NODE_ATTESTATION_KEY_BITS);
	res = TEE_AllocateTransientObject(TEE_TYPE_RSA_KEYPAIR,
					  NODE_ATTESTATION_KEY_BITS,
					  &transient);
	if (res) {
		EMSG("TEE_AllocateTransientObject: %#" PRIx32, res);
		return res;
	}

	res = TEE_GenerateKey(transient, NODE_ATTESTATION_KEY_BITS, NULL, 0);
	if (res) {
		EMSG("TEE_GenerateKey: %#" PRIx32, res);
		TEE_FreeTransientObject(transient);
		return res;
	}

	res = TEE_CreatePersistentObject(
		TEE_STORAGE_PRIVATE,
		NODE_ATT_KEY_OBJ_ID, sizeof(NODE_ATT_KEY_OBJ_ID) - 1,
		TEE_DATA_FLAG_ACCESS_READ | TEE_DATA_FLAG_ACCESS_WRITE,
		transient,
		NULL, 0,
		&pers);
	TEE_FreeTransientObject(transient);
	if (res) {
		EMSG("TEE_CreatePersistentObject: %#" PRIx32, res);
		return res;
	}

	IMSG("RSA keypair persisted to TSK-encrypted secure storage");
	*out = pers;
	return TEE_SUCCESS;
}

static TEE_Result cmd_get_pubkey(uint32_t pt, TEE_Param params[TEE_NUM_PARAMS])
{
	TEE_Result res = TEE_ERROR_GENERIC;
	TEE_ObjectHandle key = TEE_HANDLE_NULL;
	uint32_t mod_size = 0;
	uint32_t exp_size = 0;
	const uint32_t exp_pt = TEE_PARAM_TYPES(
		TEE_PARAM_TYPE_MEMREF_OUTPUT,
		TEE_PARAM_TYPE_MEMREF_OUTPUT,
		TEE_PARAM_TYPE_NONE,
		TEE_PARAM_TYPE_NONE);

	if (pt != exp_pt)
		return TEE_ERROR_BAD_PARAMETERS;

	res = open_or_create_keypair(&key);
	if (res)
		return res;

	mod_size = params[0].memref.size;
	res = TEE_GetObjectBufferAttribute(key, TEE_ATTR_RSA_MODULUS,
					   params[0].memref.buffer, &mod_size);
	if (res) {
		EMSG("Get RSA modulus: %#" PRIx32 " (need %u, got buffer of %u)",
		     res, mod_size, params[0].memref.size);
		params[0].memref.size = mod_size;
		goto out;
	}
	params[0].memref.size = mod_size;

	exp_size = params[1].memref.size;
	res = TEE_GetObjectBufferAttribute(key, TEE_ATTR_RSA_PUBLIC_EXPONENT,
					   params[1].memref.buffer, &exp_size);
	if (res) {
		EMSG("Get RSA exponent: %#" PRIx32, res);
		params[1].memref.size = exp_size;
		goto out;
	}
	params[1].memref.size = exp_size;

out:
	TEE_CloseObject(key);
	return res;
}

static TEE_Result cmd_attest(uint32_t pt, TEE_Param params[TEE_NUM_PARAMS])
{
	TEE_Result res = TEE_ERROR_GENERIC;
	TEE_ObjectHandle key = TEE_HANDLE_NULL;
	TEE_OperationHandle op = TEE_HANDLE_NULL;
	uint8_t digest[NODE_ATTESTATION_MEASUREMENT_SIZE];
	uint32_t digest_size = sizeof(digest);
	uint32_t sig_size = 0;
	const uint32_t exp_pt = TEE_PARAM_TYPES(
		TEE_PARAM_TYPE_MEMREF_INPUT,   /* nonce */
		TEE_PARAM_TYPE_MEMREF_OUTPUT,  /* measurement */
		TEE_PARAM_TYPE_MEMREF_OUTPUT,  /* signature */
		TEE_PARAM_TYPE_NONE);

	if (pt != exp_pt)
		return TEE_ERROR_BAD_PARAMETERS;
	if (params[0].memref.size != NODE_ATTESTATION_NONCE_SIZE)
		return TEE_ERROR_BAD_PARAMETERS;
	if (params[1].memref.size < NODE_ATTESTATION_MEASUREMENT_SIZE)
		return TEE_ERROR_SHORT_BUFFER;
	if (params[2].memref.size < NODE_ATTESTATION_SIGNATURE_SIZE)
		return TEE_ERROR_SHORT_BUFFER;

	/* Derive measurement = SHA-256(TA_BUILD_MARKER) — deterministic. */
	res = TEE_AllocateOperation(&op, TEE_ALG_SHA256, TEE_MODE_DIGEST, 0);
	if (res) {
		EMSG("AllocateOperation SHA256: %#" PRIx32, res);
		return res;
	}
	res = TEE_DigestDoFinal(op, TA_BUILD_MARKER,
				sizeof(TA_BUILD_MARKER) - 1,
				digest, &digest_size);
	TEE_FreeOperation(op);
	op = TEE_HANDLE_NULL;
	if (res) {
		EMSG("DigestDoFinal (measurement): %#" PRIx32, res);
		return res;
	}
	memcpy(params[1].memref.buffer, digest,
	       NODE_ATTESTATION_MEASUREMENT_SIZE);
	params[1].memref.size = NODE_ATTESTATION_MEASUREMENT_SIZE;

	/*
	 * Compute the digest the verifier will reconstruct:
	 *   to_sign_digest = SHA-256(nonce || measurement)
	 * Both nonce + measurement are 32 bytes, total 64 bytes. The verifier
	 * has both (nonce supplied by Alice, measurement returned here) and
	 * recomputes the digest before RSA-PSS verify.
	 */
	res = TEE_AllocateOperation(&op, TEE_ALG_SHA256, TEE_MODE_DIGEST, 0);
	if (res) {
		EMSG("AllocateOperation SHA256 (sign digest): %#" PRIx32, res);
		return res;
	}
	TEE_DigestUpdate(op, params[0].memref.buffer,
			 NODE_ATTESTATION_NONCE_SIZE);
	digest_size = sizeof(digest);
	res = TEE_DigestDoFinal(op, digest, NODE_ATTESTATION_MEASUREMENT_SIZE,
				digest, &digest_size);
	TEE_FreeOperation(op);
	op = TEE_HANDLE_NULL;
	if (res) {
		EMSG("DigestDoFinal (sign digest): %#" PRIx32, res);
		return res;
	}

	/* Sign the digest with RSASSA-PSS-SHA256, salt length = digest length. */
	res = open_or_create_keypair(&key);
	if (res)
		return res;

	res = TEE_AllocateOperation(&op,
				    TEE_ALG_RSASSA_PKCS1_PSS_MGF1_SHA256,
				    TEE_MODE_SIGN,
				    NODE_ATTESTATION_KEY_BITS);
	if (res) {
		EMSG("AllocateOperation RSA-PSS-SHA256: %#" PRIx32, res);
		goto out;
	}
	res = TEE_SetOperationKey(op, key);
	if (res) {
		EMSG("SetOperationKey: %#" PRIx32, res);
		goto out;
	}

	sig_size = params[2].memref.size;
	res = TEE_AsymmetricSignDigest(op, NULL, 0,
				       digest, digest_size,
				       params[2].memref.buffer, &sig_size);
	if (res) {
		EMSG("AsymmetricSignDigest: %#" PRIx32, res);
		params[2].memref.size = sig_size;
		goto out;
	}
	params[2].memref.size = sig_size;

out:
	if (op != TEE_HANDLE_NULL)
		TEE_FreeOperation(op);
	if (key != TEE_HANDLE_NULL)
		TEE_CloseObject(key);
	return res;
}

TEE_Result TA_CreateEntryPoint(void)
{
	return TEE_SUCCESS;
}

void TA_DestroyEntryPoint(void)
{
}

TEE_Result TA_OpenSessionEntryPoint(uint32_t __unused param_types,
				    TEE_Param __unused params[4],
				    void **__unused session)
{
	return TEE_SUCCESS;
}

void TA_CloseSessionEntryPoint(void *__unused session)
{
}

TEE_Result TA_InvokeCommandEntryPoint(void *__unused session, uint32_t cmd,
				      uint32_t param_types,
				      TEE_Param params[TEE_NUM_PARAMS])
{
	switch (cmd) {
	case NODE_ATTESTATION_CMD_GET_PUBKEY:
		return cmd_get_pubkey(param_types, params);
	case NODE_ATTESTATION_CMD_ATTEST:
		return cmd_attest(param_types, params);
	default:
		EMSG("Unsupported command %#" PRIx32, cmd);
		return TEE_ERROR_NOT_SUPPORTED;
	}
}
