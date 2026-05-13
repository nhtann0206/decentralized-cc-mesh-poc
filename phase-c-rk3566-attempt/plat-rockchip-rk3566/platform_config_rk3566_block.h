/*
 * Excerpt from optee_os/core/arch/arm/plat-rockchip/platform_config.h
 *
 * This is the RK3566 case block that I added to the upstream plat-rockchip
 * port. The full file is part of the OP-TEE OS source tree
 * (https://github.com/OP-TEE/optee_os, BSD-2-Clause); only the rk3566
 * region I authored is reproduced here.
 *
 * SoC: Rockchip RK3566 / RK3568 family (Cortex-A55 quad-core, GICv3).
 * Addresses cross-validated against:
 *   - trusted-firmware-a/plat/rockchip/rk3568/rk3568_def.h (same SoC family)
 *   - PoC report R-012 §III.4 (secure DRAM 0x08400000-0x0A400000,
 *     UART2 0xfe660000)
 *   - u-boot/include/configs/quartz64_rk3566.h
 */

#elif defined(PLATFORM_FLAVOR_rk3566)

#define GIC_BASE		0xfd400000
#define GIC_SIZE		SIZE_K(64)
#define GICC_BASE		0
#define GICD_BASE		GIC_BASE
#define GICR_BASE		(GIC_BASE + 0x60000)

#define UART0_BASE		0xfdd50000
#define UART0_SIZE		SIZE_K(64)

#define UART1_BASE		0xfe650000
#define UART1_SIZE		SIZE_K(64)

#define UART2_BASE		0xfe660000
#define UART2_SIZE		SIZE_K(64)

#define UART3_BASE		0xfe670000
#define UART3_SIZE		SIZE_K(64)

#define SGRF_BASE		0xfdd18000
#define SGRF_SIZE		SIZE_K(32)

#define DDRSGRF_BASE		0xfe200000
#define DDRSGRF_SIZE		SIZE_K(32)

#define GRF_BASE		0xfdc60000
#define GRF_SIZE		SIZE_K(64)

#define CRU_BASE		0xfdd20000
#define CRU_SIZE		SIZE_K(64)

#endif /* PLATFORM_FLAVOR_rk3566 */
