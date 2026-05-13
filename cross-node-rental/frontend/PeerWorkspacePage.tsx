import { useState, useCallback, useMemo, useEffect } from 'react';
import { VncScreen } from 'react-vnc';
import { Card, CardContent, CardFooter } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Badge } from '@/components/ui/badge';
import { PageLayout } from '@/components/layouts/PageLayout';
import { Input } from '@/components/ui/input';
import {
  Server, Loader2, Square, Zap, MemoryStick, HardDrive,
  ArrowRight, Monitor, Play, StopCircle, Pause, Shield,
} from 'lucide-react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { toast } from 'sonner';
import { useAuthContext } from '@/contexts/AuthContext';
import { apiClient } from '@/lib/apiClient';
import { SecureStorage } from '@/lib/secureStorage';
import {
  peerWorkspaceApi,
  type SecureComputer,
  type StartSessionRequest,
} from '@/lib/workspaceApi';
import { useProviderCcAttestation } from '@/hooks/useProviderCcAttestation';
import { CcAttestationPanel } from '@/components/CcAttestationPanel';

// ─── Peer Workspace Page ───────────────────────────────────────
// M4: Alice rents VMs from a peer (Bob) via cross-node proxy.
// Flow: enter peer node_id → browse marketplace → rent → manage.

export default function PeerWorkspacePage() {
  const { isAuthenticated } = useAuthContext();
  const queryClient = useQueryClient();
  const [peerId, setPeerId] = useState('');
  const [activePeer, setActivePeer] = useState<string | null>(null);

  // Fetch connected Lightning peers for dropdown
  const { data: knownPeers } = useQuery({
    queryKey: ['peers', 'list'],
    queryFn: () => apiClient.get<Array<{ node_id: string; is_connected: boolean }>>('/peers'),
    enabled: isAuthenticated,
  });
  const [activeSessionId, setActiveSessionId] = useState<string | null>(null);
  const [activeSessionPeer, setActiveSessionPeer] = useState<string | null>(null);

  // 4-step rent wizard: when set, the user clicked "Rent VM" on a card and
  // is on step 2 (CC verify) → step 3 (confirm) before the actual session
  // start in step 4. `null` means we're on step 1 (browse).
  const [pendingRent, setPendingRent] = useState<SecureComputer | null>(null);

  // Fetch peer computers
  const { data: computers, isLoading: loadingComputers } = useQuery({
    queryKey: ['peer-workspace', 'computers', activePeer],
    queryFn: () => peerWorkspaceApi.listComputers(activePeer!),
    enabled: !!activePeer && isAuthenticated,
  });

  // Fetch active session
  const { data: session } = useQuery({
    queryKey: ['peer-workspace', 'session', activeSessionPeer, activeSessionId],
    queryFn: () => peerWorkspaceApi.getSession(activeSessionPeer!, activeSessionId!),
    enabled: !!activeSessionId && !!activeSessionPeer && isAuthenticated,
    // Use function form — reading `session` from outer scope inside the
    // config that declares `session` would hit TDZ (ReferenceError). React
    // Query passes the query itself, so read status from `query.state.data`.
    refetchInterval: (query) => {
      if (!activeSessionId) return false;
      return query.state.data?.status === 'starting' ? 3000 : 5000;
    },
  });

  // CC attestation for the wizard's step 2. Fires only when a Rent click
  // populated `pendingRent` AND we know the peer node id. Refetch is wired
  // through to the panel's "Recheck" button so the demo can show a live
  // round-trip.
  const ccAttestation = useProviderCcAttestation(
    pendingRent && activePeer ? activePeer : undefined,
  );

  // Start session mutation
  const startMutation = useMutation({
    mutationFn: ({ nodeId, request }: { nodeId: string; request: StartSessionRequest }) =>
      peerWorkspaceApi.startSession(nodeId, request),
    onSuccess: (data) => {
      setActiveSessionId(data.session_id);
      setActiveSessionPeer(activePeer);
      setPendingRent(null);
      queryClient.invalidateQueries({ queryKey: ['my-rentals'] });
      toast.success('Session started');
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      // Strip the L402 wrapping Alice backend adds when forwarding peer errors
      // so the user sees the underlying "Insufficient credits" etc. text.
      const cleaned = msg.replace(/^.*?Peer returned \d+[^:]*:\s*/, '').replace(/^.*?"error":"([^"]+)".*$/, '$1');
      toast.error(`Rent failed: ${cleaned}`);
    },
  });

  // Stop session mutation
  const stopMutation = useMutation({
    mutationFn: ({ nodeId, sessionId }: { nodeId: string; sessionId: string }) =>
      peerWorkspaceApi.stopSession(nodeId, sessionId),
    onSuccess: () => {
      setActiveSessionId(null);
      setActiveSessionPeer(null);
      queryClient.invalidateQueries({ queryKey: ['peer-workspace'] });
      queryClient.invalidateQueries({ queryKey: ['my-rentals'] });
      toast.success('Session stopped');
    },
    onError: (err: unknown) => {
      const msg = err instanceof Error ? err.message : String(err);
      toast.error(`Stop failed: ${msg}`);
    },
  });

  const handleConnect = useCallback(() => {
    if (peerId.trim()) {
      setActivePeer(peerId.trim());
    }
  }, [peerId]);

  // Step 1 → 2: clicking Rent enters the wizard but does NOT start the
  // session yet. We wait for CC verify (step 2) and explicit confirm (step 3).
  const handleRent = useCallback((computer: SecureComputer) => {
    if (!activePeer) return;
    setPendingRent(computer);
  }, [activePeer]);

  const handleCancelRent = useCallback(() => {
    setPendingRent(null);
  }, []);

  // Step 3 → 4: user has reviewed the verified CC badge + price and clicked
  // Start. Now we actually mutate. CC verification must have succeeded
  // (button is disabled otherwise).
  const handleConfirmRent = useCallback(() => {
    if (!activePeer || !pendingRent) return;
    startMutation.mutate({
      nodeId: activePeer,
      request: {
        computer_id: pendingRent.id,
        config: {
          computer_id: pendingRent.id,
          ram_gb: pendingRent.ram_gb,
          storage_gb: pendingRent.storage_gb,
          environment: 'debian-13',
        },
      },
    });
  }, [activePeer, pendingRent, startMutation]);

  const handleRecheckCc = useCallback(() => {
    ccAttestation.refetch();
  }, [ccAttestation]);

  const handleStop = useCallback(() => {
    if (!activeSessionPeer || !activeSessionId) return;
    stopMutation.mutate({ nodeId: activeSessionPeer, sessionId: activeSessionId });
  }, [activeSessionPeer, activeSessionId, stopMutation]);

  // JWT fetched async from SecureStorage — needed as ?token=<jwt> query
  // because browser WebSocket API cannot set Authorization header.
  const [vncToken, setVncToken] = useState<string | null>(null);
  useEffect(() => {
    if (!activeSessionId) return;
    SecureStorage.getJwtToken().then((t) => setVncToken(t || ''));
  }, [activeSessionId]);

  const vncUrl = useMemo(() => {
    if (!activeSessionPeer || !activeSessionId || vncToken === null) return '';
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    return `${proto}://${window.location.host}/api/v2/workspace/peer/${activeSessionPeer}/sessions/${activeSessionId}/vnc?token=${encodeURIComponent(vncToken)}`;
  }, [activeSessionPeer, activeSessionId, vncToken]);

  const statusColor = (status: string) => {
    switch (status) {
      case 'running': return 'bg-emerald-500';
      case 'starting': return 'bg-blue-500';
      case 'paused': return 'bg-yellow-500';
      case 'stopped': return 'bg-red-500';
      case 'error': return 'bg-red-500';
      case 'available': return 'bg-emerald-500';
      case 'offline': return 'bg-gray-500';
      default: return 'bg-blue-500';
    }
  };

  return (
    <PageLayout title="Peer Workspace" subtitle="Rent VMs from Lightning peers">
      {/* Peer selector */}
      <Card className="mb-6">
        <CardContent className="pt-6">
          {/* Peer selector: dropdown of connected peers OR manual input */}
          {knownPeers && knownPeers.length > 0 ? (
            <div className="flex gap-3">
              <select
                className="flex-1 rounded-md border bg-background px-3 py-2 text-sm font-mono"
                value={peerId}
                onChange={(e) => setPeerId(e.target.value)}
              >
                <option value="">Select a connected peer...</option>
                {knownPeers.map((p) => (
                  <option key={p.node_id} value={p.node_id}>
                    {p.node_id.substring(0, 16)}... {p.is_connected ? '(connected)' : '(offline)'}
                  </option>
                ))}
              </select>
              <Button onClick={handleConnect} disabled={!peerId.trim()}>
                <ArrowRight className="w-4 h-4 mr-2" />
                Browse
              </Button>
            </div>
          ) : (
            <div className="flex gap-3">
              <Input
                placeholder="Enter peer node ID (Lightning pubkey)"
                value={peerId}
                onChange={(e) => setPeerId(e.target.value)}
                className="font-mono text-xs"
              />
              <Button onClick={handleConnect} disabled={!peerId.trim()}>
                <ArrowRight className="w-4 h-4 mr-2" />
                Browse
              </Button>
            </div>
          )}
          {activePeer && (
            <p className="mt-2 text-xs text-muted-foreground font-mono">
              Connected to: {activePeer.substring(0, 16)}...
            </p>
          )}
        </CardContent>
      </Card>

      {/* Active session */}
      {session && activeSessionId && (<>
        <Card className="mb-6 border-emerald-500/30">
          <CardContent className="pt-6">
            <div className="flex items-center justify-between mb-4">
              <div className="flex items-center gap-2">
                <Monitor className="w-5 h-5" />
                <span className="font-medium">Active Session</span>
                <Badge className={statusColor(session.status)}>
                  {session.status === 'starting' ? (
                    <><Loader2 className="w-3 h-3 mr-1 animate-spin" /> Starting...</>
                  ) : session.status === 'paused' ? (
                    <><Pause className="w-3 h-3 mr-1" /> Paused</>
                  ) : session.status === 'running' ? (
                    <><Play className="w-3 h-3 mr-1" /> Running</>
                  ) : session.status === 'error' ? (
                    <><StopCircle className="w-3 h-3 mr-1" /> Error</>
                  ) : (
                    session.status
                  )}
                </Badge>
              </div>
              <Button
                variant="destructive"
                size="sm"
                onClick={handleStop}
                disabled={stopMutation.isPending || session.status === 'stopped'}
              >
                <StopCircle className="w-4 h-4 mr-1" />
                Stop
              </Button>
            </div>
            <div className="grid grid-cols-4 gap-4 text-sm">
              <div>
                <p className="text-muted-foreground">Elapsed</p>
                <p className="font-mono">{Math.floor(session.elapsed_seconds / 60)}m {session.elapsed_seconds % 60}s</p>
              </div>
              <div>
                <p className="text-muted-foreground">Cost</p>
                <p className="font-mono">{session.cost_sats} sats</p>
              </div>
              <div>
                <p className="text-muted-foreground">Rate</p>
                <p className="font-mono">{session.price_sats_per_min} sats/min</p>
              </div>
              <div>
                <p className="text-muted-foreground">Runtime</p>
                <p className="font-mono">{session.runtime_type}</p>
              </div>
            </div>
            {session.bolt12_offer && (
              <div className="mt-3 p-2 bg-muted rounded text-xs font-mono break-all">
                <span className="text-muted-foreground">BOLT12 Offer: </span>
                {session.bolt12_offer.substring(0, 40)}...
              </div>
            )}
          </CardContent>
        </Card>

        {/* VM Starting indicator */}
        {session.status === 'starting' && (
          <Card className="mb-6 border-blue-500/30">
            <CardContent className="pt-6">
              <div className="flex flex-col items-center justify-center p-12 text-muted-foreground">
                <Loader2 className="w-10 h-10 animate-spin mb-4 text-blue-500" />
                <p className="font-medium text-foreground">Starting VM...</p>
                <p className="text-sm mt-1">Booting desktop environment. This may take 1-2 minutes.</p>
              </div>
            </CardContent>
          </Card>
        )}

        {/* VM Desktop Viewer (noVNC) */}
        {session.status === 'running' && activeSessionPeer && activeSessionId && (
          <Card className="mb-6">
            <CardContent className="pt-6">
              <div className="flex items-center justify-between mb-3">
                <div className="flex items-center gap-2">
                  <Monitor className="w-5 h-5" />
                  <span className="font-medium">VM Desktop</span>
                </div>
              </div>
              <div className="border rounded-lg overflow-hidden bg-black" style={{ height: '600px' }}>
                <VncScreen
                  url={vncUrl}
                  scaleViewport
                  focusOnClick
                  background="#000000"
                  style={{ width: '100%', height: '100%' }}
                  onConnect={() => console.log('VNC connected')}
                  onDisconnect={() => console.log('VNC disconnected')}
                />
              </div>
              <p className="mt-2 text-xs text-muted-foreground">
                Interact with the VM desktop directly. Chromium browser and terminal are available.
              </p>
            </CardContent>
          </Card>
        )}
      </>)}

      {/* Rent wizard — shown after Rent click (step 2 + 3 of the 4-step
          flow). Step 1 is the marketplace grid below; step 4 is the active
          session block above. */}
      {pendingRent && activePeer && !activeSessionId && (
        <Card className="mb-6 border-primary/40">
          <CardContent className="pt-6 space-y-4">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-2">
                <Shield className="w-5 h-5 text-primary" />
                <span className="font-medium">Rent confirmation — Confidential Compute proof</span>
              </div>
              <span className="text-xs text-muted-foreground font-mono">
                Peer {activePeer.substring(0, 12)}…
              </span>
            </div>

            <CcAttestationPanel
              data={ccAttestation.data}
              isLoading={ccAttestation.isLoading}
              isFetching={ccAttestation.isFetching}
              error={ccAttestation.error}
              onRecheck={handleRecheckCc}
            />

            <div className="grid grid-cols-2 md:grid-cols-4 gap-3 rounded-md border bg-muted/40 px-3 py-2 text-sm">
              <div>
                <p className="text-muted-foreground text-xs">Machine</p>
                <p className="font-medium">{pendingRent.machine_type}</p>
              </div>
              <div>
                <p className="text-muted-foreground text-xs">Rate</p>
                <p className="font-mono">{pendingRent.price_sats_per_min} sats/min</p>
              </div>
              <div>
                <p className="text-muted-foreground text-xs">RAM</p>
                <p className="font-mono">{pendingRent.ram_gb} GB</p>
              </div>
              <div>
                <p className="text-muted-foreground text-xs">Storage</p>
                <p className="font-mono">{pendingRent.storage_gb} GB</p>
              </div>
            </div>

            <div className="flex justify-end gap-2">
              <Button
                variant="outline"
                onClick={handleCancelRent}
                disabled={startMutation.isPending}
              >
                Cancel
              </Button>
              <Button
                onClick={handleConfirmRent}
                disabled={
                  !ccAttestation.data ||
                  !!ccAttestation.error ||
                  ccAttestation.isLoading ||
                  ccAttestation.isFetching ||
                  startMutation.isPending
                }
              >
                {startMutation.isPending ? (
                  <Loader2 className="w-4 h-4 animate-spin mr-2" />
                ) : (
                  <Play className="w-4 h-4 mr-2" />
                )}
                Start session
              </Button>
            </div>
          </CardContent>
        </Card>
      )}

      {/* Marketplace */}
      {loadingComputers && (
        <div className="flex items-center justify-center p-8">
          <Loader2 className="w-6 h-6 animate-spin mr-2" />
          Loading peer marketplace...
        </div>
      )}

      {computers && computers.length > 0 && (
        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {computers.map((computer) => (
            <Card key={computer.id} className="hover:border-primary/50 transition-colors">
              <CardContent className="pt-6">
                <div className="flex items-center justify-between mb-3">
                  <div className="flex items-center gap-2">
                    <Server className="w-5 h-5 text-primary" />
                    <span className="font-medium">{computer.machine_type}</span>
                  </div>
                  <Badge className={statusColor(computer.status)}>
                    {computer.status}
                  </Badge>
                </div>
                <div className="space-y-2 text-sm text-muted-foreground">
                  <div className="flex items-center gap-2">
                    <Zap className="w-4 h-4" />
                    <span>{computer.price_sats_per_min} sats/min</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <MemoryStick className="w-4 h-4" />
                    <span>{computer.ram_gb} GB RAM</span>
                  </div>
                  <div className="flex items-center gap-2">
                    <HardDrive className="w-4 h-4" />
                    <span>{computer.storage_gb} GB Storage</span>
                  </div>
                </div>
              </CardContent>
              <CardFooter>
                <Button
                  className="w-full"
                  disabled={computer.status !== 'available' || startMutation.isPending || !!activeSessionId}
                  onClick={() => handleRent(computer)}
                >
                  {startMutation.isPending ? (
                    <Loader2 className="w-4 h-4 animate-spin mr-2" />
                  ) : (
                    <Square className="w-4 h-4 mr-2" />
                  )}
                  Rent VM
                </Button>
              </CardFooter>
            </Card>
          ))}
        </div>
      )}

      {computers && computers.length === 0 && (
        <div className="text-center p-8 text-muted-foreground">
          No VMs available on this peer.
        </div>
      )}

      {!activePeer && !activeSessionId && (
        <div className="text-center p-12 text-muted-foreground">
          <Server className="w-12 h-12 mx-auto mb-4 opacity-50" />
          <p>Enter a peer's Lightning node ID to browse their VM marketplace.</p>
        </div>
      )}
    </PageLayout>
  );
}
