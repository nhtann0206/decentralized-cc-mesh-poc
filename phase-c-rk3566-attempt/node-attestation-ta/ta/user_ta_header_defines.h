/* SPDX-License-Identifier: BSD-2-Clause */
/*
 * GlobalPlatform-style TA properties for node-attestation. The header
 * filename is dictated by the OP-TEE TA dev kit and must not change.
 */

#ifndef USER_TA_HEADER_DEFINES_H
#define USER_TA_HEADER_DEFINES_H

#include <node_attestation_ta.h>

#define TA_UUID NODE_ATTESTATION_UUID

/*
 * SINGLE_INSTANCE + MULTI_SESSION: the TA holds one persistent keypair,
 * but multiple normal-world clients may verify in parallel.
 */
#define TA_FLAGS \
	(TA_FLAG_SINGLE_INSTANCE | TA_FLAG_MULTI_SESSION)

/*
 * Stack and heap sizes. RSA-2048 keygen + signing fit comfortably in
 * 8 KB stack / 64 KB heap. The chip has 1 GB DRAM and OP-TEE TZDRAM
 * is 32 MB so we have plenty of headroom.
 */
#define TA_STACK_SIZE (8 * 1024)
#define TA_DATA_SIZE (64 * 1024)

#define TA_VERSION "1.0"
#define TA_DESCRIPTION \
	"NodeOS attestation TA — RSA-PSS sign of (nonce || measurement) " \
	"with HUK-derived persistent keypair"

#endif /* USER_TA_HEADER_DEFINES_H */
