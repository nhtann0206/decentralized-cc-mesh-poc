// SPDX-License-Identifier: BSD-2-Clause
/*
 * Copyright (C) 2026, Tan Nguyen Huu <nguyenhuutan262004@gmail.com>
 * Originally authored as part of a 2026 NodeOS contract; published here
 * under BSD-2-Clause as a portfolio extract.
 *
 * Initial port for Rockchip RK3566 (Radxa Zero 3W). Phase C — minimal
 * compile-clean shell. Hardware-backed HUK and runtime TZASC region updates
 * are stubbed; BL31 sets up the static secure DRAM region (0x08400000 -
 * 0x0A400000, 32 MB) at boot per R-012 §III.4. The default upstream weak
 * symbols for tee_otp_get_hw_unique_key and hw_get_random_bytes apply until
 * Day 1+ wires the RK3566 OTP/TRNG drivers.
 *
 * RK3566 SoC family is shared with RK3568 — addresses match upstream
 * trusted-firmware-a/plat/rockchip/rk3568/rk3568_def.h.
 */

#include <common.h>
#include <io.h>
#include <kernel/panic.h>
#include <mm/core_memprot.h>
#include <platform.h>
#include <platform_config.h>

/*
 * Stub. BL31 (TF-A) configures SGRF firewall slv_con registers at boot to
 * mark the secure DDR window 0x08400000 - 0x0A400000 (CFG_TZDRAM) as
 * Secure-only. We do not need runtime region updates for the Phase C PoC;
 * the static window is set once and persists for the device lifetime.
 *
 * Day 1+ TODO: if dynamic region updates become required (e.g. for plug-in
 * TAs that need their own protected RAM), implement via SGRF_FIREWALL_*
 * registers per the RK3568 TRM. Reference: trusted-firmware-a/plat/
 * rockchip/rk3568/drivers/soc/soc.c uses
 * SGRF_FIREWALL_SLV_CON(i) at SGRF_BASE + 0x240 + i*4.
 */
int platform_secure_ddr_region(int rgn __unused, paddr_t st __unused,
			       size_t sz __unused)
{
	return 0;
}
