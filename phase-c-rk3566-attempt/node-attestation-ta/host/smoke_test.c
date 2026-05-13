/* SPDX-License-Identifier: BSD-2-Clause */
/*
 * NodeOS attestation TA — smoke test client.
 *
 * Minimal libteec client to confirm the TA loads + the two commands
 * (GET_PUBKEY, ATTEST) round-trip on Radxa Zero 3W with the Rockchip-
 * shipped BL32. This is throw-away validation before the full Rust
 * libteec FFI binding (Pipeline L) gets wired into the Bob backend.
 *
 * Build on Radxa:
 *   gcc -o /tmp/optee_smoke smoke_test.c -lteec
 *
 * Run:
 *   /tmp/optee_smoke
 *   # First run: ~30s (RSA-2048 keygen). Subsequent: <1s.
 *
 * Output: hex dumps of the public modulus + the signed attestation,
 * or `TEEC_*` error code on failure.
 */

#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <tee_client_api.h>

#define UUID_INIT \
	{ 0xfeb95976, 0x43e8, 0x4e43, \
		{ 0xa7, 0x9c, 0x51, 0x03, 0x59, 0xff, 0xf6, 0x33 } }

#define CMD_GET_PUBKEY 0x1000
#define CMD_ATTEST 0x1001

#define NONCE_SIZE 32
#define MEASUREMENT_SIZE 32
#define SIG_SIZE 256
#define MOD_SIZE 256
#define EXP_SIZE 8

static void hexdump(const char *label, const uint8_t *buf, size_t len)
{
	printf("%s (%zu bytes):\n  ", label, len);
	for (size_t i = 0; i < len; i++) {
		printf("%02x", buf[i]);
		if ((i + 1) % 32 == 0 && i + 1 < len)
			printf("\n  ");
	}
	printf("\n");
}

int main(void)
{
	TEEC_Result res;
	TEEC_Context ctx;
	TEEC_Session sess;
	TEEC_Operation op;
	TEEC_UUID uuid = UUID_INIT;
	uint32_t err_origin = 0;

	uint8_t modulus[MOD_SIZE] = { 0 };
	uint8_t exponent[EXP_SIZE] = { 0 };
	uint8_t nonce[NONCE_SIZE] = {
		/* deterministic test nonce so output is reproducible */
		0x01, 0x23, 0x45, 0x67, 0x89, 0xab, 0xcd, 0xef,
		0xfe, 0xdc, 0xba, 0x98, 0x76, 0x54, 0x32, 0x10,
		0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0xa6, 0xa7,
		0xb0, 0xb1, 0xb2, 0xb3, 0xb4, 0xb5, 0xb6, 0xb7
	};
	uint8_t measurement[MEASUREMENT_SIZE] = { 0 };
	uint8_t signature[SIG_SIZE] = { 0 };

	res = TEEC_InitializeContext(NULL, &ctx);
	if (res != TEEC_SUCCESS) {
		fprintf(stderr, "TEEC_InitializeContext: 0x%x\n", res);
		return 1;
	}

	res = TEEC_OpenSession(&ctx, &sess, &uuid, TEEC_LOGIN_PUBLIC,
			       NULL, NULL, &err_origin);
	if (res != TEEC_SUCCESS) {
		fprintf(stderr, "TEEC_OpenSession: 0x%x (origin %u)\n",
			res, err_origin);
		TEEC_FinalizeContext(&ctx);
		return 1;
	}
	printf("Session opened to TA feb95976-43e8-4e43-a79c-510359fff633\n");

	/* CMD_GET_PUBKEY */
	memset(&op, 0, sizeof(op));
	op.paramTypes = TEEC_PARAM_TYPES(
		TEEC_MEMREF_TEMP_OUTPUT, TEEC_MEMREF_TEMP_OUTPUT,
		TEEC_NONE, TEEC_NONE);
	op.params[0].tmpref.buffer = modulus;
	op.params[0].tmpref.size = sizeof(modulus);
	op.params[1].tmpref.buffer = exponent;
	op.params[1].tmpref.size = sizeof(exponent);

	res = TEEC_InvokeCommand(&sess, CMD_GET_PUBKEY, &op, &err_origin);
	if (res != TEEC_SUCCESS) {
		fprintf(stderr, "CMD_GET_PUBKEY: 0x%x (origin %u)\n",
			res, err_origin);
		goto out;
	}
	printf("\n=== CMD_GET_PUBKEY OK ===\n");
	hexdump("Modulus", modulus, op.params[0].tmpref.size);
	hexdump("Exponent", exponent, op.params[1].tmpref.size);

	/* CMD_ATTEST */
	memset(&op, 0, sizeof(op));
	op.paramTypes = TEEC_PARAM_TYPES(
		TEEC_MEMREF_TEMP_INPUT,  /* nonce */
		TEEC_MEMREF_TEMP_OUTPUT, /* measurement */
		TEEC_MEMREF_TEMP_OUTPUT, /* signature */
		TEEC_NONE);
	op.params[0].tmpref.buffer = nonce;
	op.params[0].tmpref.size = NONCE_SIZE;
	op.params[1].tmpref.buffer = measurement;
	op.params[1].tmpref.size = sizeof(measurement);
	op.params[2].tmpref.buffer = signature;
	op.params[2].tmpref.size = sizeof(signature);

	res = TEEC_InvokeCommand(&sess, CMD_ATTEST, &op, &err_origin);
	if (res != TEEC_SUCCESS) {
		fprintf(stderr, "CMD_ATTEST: 0x%x (origin %u)\n",
			res, err_origin);
		goto out;
	}
	printf("\n=== CMD_ATTEST OK ===\n");
	hexdump("Nonce (input)", nonce, NONCE_SIZE);
	hexdump("Measurement (output)", measurement,
		op.params[1].tmpref.size);
	hexdump("Signature (output)", signature,
		op.params[2].tmpref.size);

out:
	TEEC_CloseSession(&sess);
	TEEC_FinalizeContext(&ctx);
	return res == TEEC_SUCCESS ? 0 : 1;
}
