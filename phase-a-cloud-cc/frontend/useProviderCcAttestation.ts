import { useQuery } from '@tanstack/react-query';
import { peerWorkspaceApi, type CcAttestation } from '@/lib/workspaceApi';

/**
 * Phase A — fetch + verify the provider's CC attestation BEFORE renting.
 *
 * The request goes Alice frontend → Alice backend → (L402) → Bob's
 * `/external/workspace/cc-attestation`. Alice's backend verifies the AMD
 * chain locally (zero-trust) and returns the verified DTO. A successful
 * query means Bob is provably running on a CC platform — the green badge
 * shown in step 2 of the rent wizard.
 *
 * Cached per-peer for 5 min; manual refetch via `Recheck` triggers a fresh
 * round-trip for demo-time live verification.
 */
export function useProviderCcAttestation(
  peerNodeId: string | undefined,
  options?: { enabled?: boolean },
) {
  return useQuery<CcAttestation, Error>({
    queryKey: ['workspace', 'peer', peerNodeId, 'cc-attestation'],
    queryFn: () => peerWorkspaceApi.getProviderCcAttestation(peerNodeId as string),
    enabled: !!peerNodeId && (options?.enabled ?? true),
    staleTime: 5 * 60_000,
    retry: 1,
  });
}
