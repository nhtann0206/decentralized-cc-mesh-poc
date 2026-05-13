ifeq ($(PLATFORM_FLAVOR),rk3566)
include core/arch/arm/cpu/cortex-armv8-0.mk
$(call force,CFG_TEE_CORE_NB_CORE,4)
$(call force,CFG_ARM_GICV3,y)
CFG_CRYPTO_WITH_CE ?= y

# Secure DRAM region per R-012 §III.4 — BL31 reserves 0x08400000-0x0A400000
# (32 MB) and configures TZASC; BL32 (this OP-TEE OS) lives inside it.
# Shared memory window placed immediately after for renter/TA traffic.
CFG_TZDRAM_START ?= 0x08400000
CFG_TZDRAM_SIZE  ?= 0x02000000
CFG_SHMEM_START  ?= 0x0a400000
CFG_SHMEM_SIZE   ?= 0x00400000

# Console: UART2 at 1.5 Mbaud (matches Linux earlycon=uart8250,mmio32,
# 0xfe660000 + console=ttyS2,1500000n8 per R-012). Same UART is shared
# with NW Linux post-handoff; OP-TEE only emits early-boot debug.
# CFG_EARLY_CONSOLE=y enables boot-time IMSG visibility on UART2 BEFORE
# DT-based console init — without it the BL31→BL32 handoff window is
# silent and we cannot tell where a panic happens. Required for any
# debug iteration on real Radxa hardware.
CFG_EARLY_CONSOLE = y
CFG_EARLY_CONSOLE_BASE ?= UART2_BASE
CFG_EARLY_CONSOLE_SIZE ?= UART2_SIZE
CFG_EARLY_CONSOLE_BAUDRATE ?= 1500000
CFG_EARLY_CONSOLE_CLK_IN_HZ ?= 24000000
endif
