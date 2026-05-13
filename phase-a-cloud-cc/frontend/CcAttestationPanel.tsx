import { useCallback, useMemo } from 'react';
import type { CcAttestation } from '@/lib/workspaceApi';

/**
 * Confidential Compute attestation detail panel — provider-polymorphic.
 *
 * Pure presentational: caller supplies the verified evidence + query state.
 * Used in the 4-step rent wizard (step 2 — verify CC, step 3 — confirm with
 * badge). Switches on `data.provider` to render the correct field set:
 *   - `amd-sev-snp` (Phase A): chip_id, SHA-384 measurement, VMPL, policy, TCB
 *   - `optee-attestation` (Phase C): TA UUID, SHA-256 measurement, BL31/BL32
 */
interface CcAttestationPanelProps {
  data: CcAttestation | undefined;
  isLoading?: boolean;
  isFetching?: boolean;
  error?: Error | null;
  onRecheck?: () => void;
  className?: string;
}

const truncateHex = (hex: string, head = 8, tail = 8): string =>
  hex.length <= head + tail + 3 ? hex : `${hex.slice(0, head)}…${hex.slice(-tail)}`;

const providerLabel = (provider: CcAttestation['provider']): string => {
  switch (provider) {
    case 'amd-sev-snp':
      return 'AMD SEV-SNP';
    case 'optee-attestation':
      return 'OP-TEE Attestation';
  }
};

const verifyingLabel = (provider: CcAttestation['provider'] | undefined): string => {
  if (provider === 'optee-attestation') return 'Verifying with OP-TEE PTA…';
  return 'Verifying with AMD…';
};

export function CcAttestationPanel({
  data,
  isLoading = false,
  isFetching = false,
  error = null,
  onRecheck,
  className,
}: CcAttestationPanelProps) {
  const handleRecheck = useCallback(() => {
    onRecheck?.();
  }, [onRecheck]);

  const state = useMemo(() => {
    if (isLoading || isFetching) {
      return {
        label: verifyingLabel(data?.provider),
        icon: '⏳',
        tone: 'border-amber-300 bg-amber-50 text-amber-900',
      };
    }
    if (error) {
      return {
        label: 'CC verification failed',
        icon: '⚠',
        tone: 'border-red-300 bg-red-50 text-red-900',
      };
    }
    if (data) {
      return {
        label: `CC Verified · ${providerLabel(data.provider)}`,
        icon: '✓',
        tone: 'border-emerald-300 bg-emerald-50 text-emerald-900',
      };
    }
    return {
      label: 'CC status unknown',
      icon: '?',
      tone: 'border-slate-300 bg-slate-50 text-slate-900',
    };
  }, [isLoading, isFetching, error, data]);

  return (
    <div
      className={`rounded-md border px-3 py-2 text-sm shadow-sm ${state.tone} ${className ?? ''}`}
    >
      <div className="flex items-center gap-2">
        <span aria-hidden className="text-lg">
          {state.icon}
        </span>
        <span className="font-semibold">{state.label}</span>
        {onRecheck && (
          <button
            type="button"
            onClick={handleRecheck}
            disabled={isFetching}
            className="ml-auto rounded border border-current px-2 py-0.5 text-xs disabled:opacity-50"
          >
            Recheck
          </button>
        )}
      </div>

      {data?.provider === 'amd-sev-snp' && (
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 font-mono text-xs">
          <dt className="opacity-60">Chip</dt>
          <dd>{truncateHex(data.chip_id_hex)}</dd>
          <dt className="opacity-60">Measurement</dt>
          <dd>{truncateHex(data.measurement_hex, 12, 12)}</dd>
          <dt className="opacity-60">VMPL</dt>
          <dd>{data.vmpl}</dd>
          <dt className="opacity-60">Policy</dt>
          <dd>{data.policy_hex}</dd>
          <dt className="opacity-60">TCB</dt>
          <dd>
            bl={data.current_tcb.bootloader} tee={data.current_tcb.tee}{' '}
            snp={data.current_tcb.snp} uc={data.current_tcb.microcode}
            {data.current_tcb.fmc !== undefined && ` fmc=${data.current_tcb.fmc}`}
          </dd>
        </dl>
      )}

      {data?.provider === 'optee-attestation' && (
        <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-2 gap-y-0.5 font-mono text-xs">
          <dt className="opacity-60">TA</dt>
          <dd>{truncateHex(data.ta_uuid_hex, 8, 8)}</dd>
          <dt className="opacity-60">Measurement</dt>
          <dd>{truncateHex(data.measurement_hex, 12, 12)}</dd>
          <dt className="opacity-60">Nonce</dt>
          <dd>{truncateHex(data.nonce_hex, 8, 8)}</dd>
          <dt className="opacity-60">RSA modulus</dt>
          <dd>{truncateHex(data.pubkey_modulus_hex, 12, 12)}</dd>
          <dt className="opacity-60">Firmware</dt>
          <dd>
            BL31={data.bl31_version} BL32={data.bl32_version}
          </dd>
        </dl>
      )}

      {error && (
        <p className="mt-2 break-words text-xs">
          {error.message ?? 'unknown error'}
        </p>
      )}
    </div>
  );
}
