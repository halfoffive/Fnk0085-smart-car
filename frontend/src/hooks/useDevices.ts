// 设备列表轮询 hook：每 2s 调用 GET /api/devices

import { useEffect, useRef, useState } from 'react';
import { getDevices } from '../lib/api';
import { DEVICE_POLL_INTERVAL_MS } from '../lib/constants';
import type { Device } from '../types/protocol';

export interface UseDevicesResult {
  devices: Device[];
  loading: boolean;
  error: string | null;
  refresh: () => void;
}

export function useDevices(): UseDevicesResult {
  const [devices, setDevices] = useState<Device[]>([]);
  const [loading, setLoading] = useState<boolean>(true);
  const [error, setError] = useState<string | null>(null);
  const abortRef = useRef<AbortController | null>(null);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const stoppedRef = useRef(false);

  const tick = async () => {
    if (stoppedRef.current) return;
    abortRef.current?.abort();
    const ac = new AbortController();
    abortRef.current = ac;
    try {
      const list = await getDevices(ac.signal);
      if (stoppedRef.current) return;
      setDevices(list);
      setError(null);
    } catch (e) {
      if (stoppedRef.current || (e as Error).name === 'AbortError') return;
      setError((e as Error).message);
    } finally {
      if (!stoppedRef.current) setLoading(false);
    }
  };

  const scheduleNext = () => {
    if (stoppedRef.current) return;
    timerRef.current = setTimeout(async () => {
      await tick();
      scheduleNext();
    }, DEVICE_POLL_INTERVAL_MS);
  };

  const refresh = () => {
    void tick();
  };

  useEffect(() => {
    stoppedRef.current = false;
    void tick();
    scheduleNext();
    return () => {
      stoppedRef.current = true;
      abortRef.current?.abort();
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, []);

  return { devices, loading, error, refresh };
}
