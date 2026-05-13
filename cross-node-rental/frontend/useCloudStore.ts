import { useCallback, useEffect, useRef, useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { useAuthContext } from '@/contexts/AuthContext';
import {
  workspaceApi,
  type SecureComputer,
  type SessionStatus,
  type BackupInfo,
} from '@/lib/workspaceApi';

const VALID_SESSION_STATUSES: Set<string> = new Set([
  'starting', 'running', 'pausing', 'paused', 'resuming', 'stopping', 'stopped', 'error',
]);

function parseSessionStatus(value: string): SessionStatus {
  return VALID_SESSION_STATUSES.has(value) ? (value as SessionStatus) : 'error';
}

// ─── Types ──────────────────────────────────────────────────────

export type StoreStep = 'browse' | 'confirm' | 'running';

// ─── Query Keys ─────────────────────────────────────────────────

const STORE_KEYS = {
  computers: ['store', 'computers'] as const,
  session: (id: string) => ['store', 'session', id] as const,
};

// ─── Hook ───────────────────────────────────────────────────────

export function useCloudStore() {
  const { isAuthenticated } = useAuthContext();
  const [step, setStep] = useState<StoreStep>('browse');
  const [selectedComputer, setSelectedComputer] = useState<SecureComputer | null>(null);
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [sessionStatus, setSessionStatus] = useState<SessionStatus>('starting');
  const [elapsedSeconds, setElapsedSeconds] = useState(0);
  const [costSats, setCostSats] = useState(0);
  const [balanceSats, setBalanceSats] = useState<number | null>(null);
  const [accessUrl, setAccessUrl] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [backups, setBackups] = useState<BackupInfo[]>([]);
  const [isBackingUp, setIsBackingUp] = useState(false);
  const isBackingUpRef = useRef(false);
  const timerRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const syncRef = useRef<ReturnType<typeof setInterval> | null>(null);

  // Fetch credit balance
  const { data: balanceData } = useQuery({
    queryKey: ['store', 'balance'],
    queryFn: () => workspaceApi.getBalance(),
    enabled: isAuthenticated,
    staleTime: 10_000,
  });

  // Keep balanceSats in sync with query data
  useEffect(() => {
    if (balanceData) setBalanceSats(balanceData.balance_sats);
  }, [balanceData]);

  // Fetch available machines
  const { data: computers = [], isLoading: isLoadingComputers } = useQuery({
    queryKey: STORE_KEYS.computers,
    queryFn: () => workspaceApi.listComputers(),
    enabled: isAuthenticated,
    staleTime: 30_000,
  });

  // Select machine → go to confirm
  const selectComputer = useCallback((computer: SecureComputer) => {
    setSelectedComputer(computer);
    setStep('confirm');
  }, []);

  // Go back to browse
  const goBack = useCallback(() => {
    if (step === 'confirm') {
      setStep('browse');
    }
  }, [step]);

  // Launch session
  const launchSession = useCallback(async () => {
    if (!selectedComputer) return;
    setSessionStatus('starting');
    setElapsedSeconds(0);
    setCostSats(0);
    setAccessUrl(null);
    setError(null);
    setStep('running');

    try {
      const response = await workspaceApi.startSession({
        computer_id: selectedComputer.id,
        config: {
          computer_id: selectedComputer.id,
          ram_gb: selectedComputer.ram_gb,
          storage_gb: selectedComputer.storage_gb,
          environment: 'default',
        },
      });
      setSessionId(response.session_id);
      setSessionStatus(parseSessionStatus(response.status));

      // Fetch session to get access_url AND init timer from backend values
      try {
        const session = await workspaceApi.getSession(response.session_id);
        setAccessUrl(session.access_url ?? null);
        // Initialize timer from backend (authoritative billing start)
        setElapsedSeconds(session.elapsed_seconds);
        setCostSats(session.cost_sats);
      } catch {
        // Non-critical — access URL may not be ready yet
      }
    } catch (e) {
      setSessionStatus('error');
      setError(e instanceof Error ? e.message : 'Failed to start machine');
    }
  }, [selectedComputer]);

  // Poll session for access_url when running but no URL yet
  useEffect(() => {
    if (sessionStatus !== 'running' || !sessionId || accessUrl) return;

    let cancelled = false;
    const pollInterval = setInterval(async () => {
      try {
        const session = await workspaceApi.getSession(sessionId);
        if (!cancelled && session.access_url) {
          setAccessUrl(session.access_url);
          clearInterval(pollInterval);
        }
      } catch {
        // Ignore polling errors
      }
    }, 5_000);

    return () => {
      cancelled = true;
      clearInterval(pollInterval);
    };
  }, [sessionStatus, sessionId, accessUrl]);

  // Cost timer — separate elapsed and cost updates to keep state updaters pure
  const priceRef = useRef(0);
  priceRef.current = selectedComputer?.price_sats_per_min ?? 0;

  useEffect(() => {
    if (sessionStatus === 'running' && selectedComputer) {
      timerRef.current = setInterval(() => {
        setElapsedSeconds(prev => {
          const next = prev + 1;
          setCostSats(Math.ceil((next / 60) * priceRef.current));
          return next;
        });
      }, 1000);
    }

    return () => {
      if (timerRef.current) {
        clearInterval(timerRef.current);
        timerRef.current = null;
      }
    };
  }, [sessionStatus, selectedComputer]);

  // Periodic sync with backend every 30s (reconcile timer drift)
  useEffect(() => {
    if (sessionStatus !== 'running' || !sessionId) return;

    syncRef.current = setInterval(async () => {
      try {
        const session = await workspaceApi.getSession(sessionId);
        setElapsedSeconds(session.elapsed_seconds);
        setCostSats(session.cost_sats);
      } catch {
        // Non-critical — local timer continues
      }
    }, 30_000);

    return () => {
      if (syncRef.current) {
        clearInterval(syncRef.current);
        syncRef.current = null;
      }
    };
  }, [sessionStatus, sessionId]);

  // Stop session
  const stopSession = useCallback(async () => {
    if (!sessionId) return;
    setSessionStatus('stopping');
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
    if (syncRef.current) {
      clearInterval(syncRef.current);
      syncRef.current = null;
    }

    try {
      const response = await workspaceApi.stopSession(sessionId);
      setSessionStatus(parseSessionStatus(response.status));
      // Use backend's authoritative settled values
      setCostSats(response.cost_sats);
      setElapsedSeconds(response.elapsed_seconds);
      setBalanceSats(response.balance_sats);
      setError(null);
    } catch (e) {
      setSessionStatus('error');
      setError(e instanceof Error ? e.message : 'Failed to stop machine');
    }
  }, [sessionId]);

  // Open desktop in new tab
  const openDesktop = useCallback(() => {
    if (accessUrl) {
      window.open(accessUrl, '_blank', 'noopener,noreferrer');
    }
  }, [accessUrl]);

  // Create backup
  const createBackup = useCallback(async () => {
    if (!sessionId || isBackingUpRef.current) return;
    isBackingUpRef.current = true;
    setIsBackingUp(true);
    try {
      await workspaceApi.createBackup(sessionId);
      const list = await workspaceApi.listBackups(sessionId);
      setBackups(list);
    } catch {
      // Backup failed — user sees no update
    } finally {
      isBackingUpRef.current = false;
      setIsBackingUp(false);
    }
  }, [sessionId]);

  // Load backups
  const loadBackups = useCallback(async () => {
    if (!sessionId) return;
    try {
      const list = await workspaceApi.listBackups(sessionId);
      setBackups(list);
    } catch {
      // Ignore
    }
  }, [sessionId]);

  // Restore from backup — creates new session from snapshot
  const restoreBackup = useCallback(async (snapshotName: string) => {
    if (!sessionId || !selectedComputer) return;
    setSessionStatus('starting');
    setElapsedSeconds(0);
    setCostSats(0);
    setAccessUrl(null);
    setError(null);
    setStep('running');

    try {
      const response = await workspaceApi.restoreBackup(sessionId, snapshotName);
      setSessionId(response.new_session_id);
      setSessionStatus(parseSessionStatus(response.status));
    } catch (e) {
      setSessionStatus('error');
      setError(e instanceof Error ? e.message : 'Failed to restore from backup');
    }
  }, [sessionId, selectedComputer]);

  // Reset to browse
  const reset = useCallback(() => {
    setStep('browse');
    setSelectedComputer(null);
    setSessionId(null);
    setSessionStatus('starting');
    setElapsedSeconds(0);
    setCostSats(0);
    setAccessUrl(null);
    setError(null);
    setBackups([]);
    setIsBackingUp(false);
    isBackingUpRef.current = false;
    if (timerRef.current) {
      clearInterval(timerRef.current);
      timerRef.current = null;
    }
    if (syncRef.current) {
      clearInterval(syncRef.current);
      syncRef.current = null;
    }
  }, []);

  return {
    step,
    selectedComputer,
    sessionId,
    sessionStatus,
    elapsedSeconds,
    costSats,
    balanceSats,
    accessUrl,
    error,
    computers,
    isLoadingComputers,
    selectComputer,
    goBack,
    launchSession,
    stopSession,
    openDesktop,
    createBackup,
    loadBackups,
    backups,
    isBackingUp,
    restoreBackup,
    reset,
  };
}
