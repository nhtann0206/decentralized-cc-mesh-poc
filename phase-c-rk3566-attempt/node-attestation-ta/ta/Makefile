# NodeOS attestation TA — Phase C build entry.
#
# Requires the OP-TEE TA dev kit produced by an OP-TEE OS build. We re-use
# the dev kit emitted by the rk3566 plat-rockchip port even though the
# resulting TA runs against the Rockchip-shipped BL32 (v2.15) — the dev
# kit headers + signing scripts are TA-ABI-compatible across BL32 versions
# because TAs target the GlobalPlatform Internal API, not OP-TEE OS internals.
#
# Build:
#   make TA_DEV_KIT_DIR=/path/to/optee_os/out/rk3566/export-ta_arm64 \
#        CROSS_COMPILE=aarch64-linux-gnu-
#
# Output: feb95976-43e8-4e43-a79c-510359fff633.{stripped.elf,ta}
# Deploy: copy the .ta file to /lib/optee_armtz/ on the target.

CFG_TEE_TA_LOG_LEVEL ?= 4

# Pin the GP Internal API compat level we coded against. v1.1 is the
# default for current optee_os — bump only after audit.
CFG_TA_OPTEE_CORE_API_COMPAT_1_1=y

# UUID — must match NODE_ATTESTATION_UUID in include/node_attestation_ta.h.
BINARY=feb95976-43e8-4e43-a79c-510359fff633

-include $(TA_DEV_KIT_DIR)/mk/ta_dev_kit.mk

ifeq ($(wildcard $(TA_DEV_KIT_DIR)/mk/ta_dev_kit.mk), )
clean:
	@echo 'TA_DEV_KIT_DIR not set or invalid: $(TA_DEV_KIT_DIR)'
	@echo 'Build the OP-TEE OS first to produce the dev kit.'
endif
