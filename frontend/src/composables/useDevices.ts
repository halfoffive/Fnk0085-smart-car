// 设备列表轮询 composable：每 2s 调用 GET /api/devices

import { onBeforeUnmount, onMounted, ref, type Ref } from 'vue';
import { getDevices } from '../lib/api';
import { DEVICE_POLL_INTERVAL_MS } from '../lib/constants';
import type { Device } from '../types/protocol';

export interface UseDevicesResult {
  devices: Ref<Device[]>;
  loading: Ref<boolean>;
  error: Ref<string | null>;
  refresh: () => void;
}

export function useDevices(): UseDevicesResult {
  const devices = ref<Device[]>([]);
  const loading = ref<boolean>(true);
  const error = ref<string | null>(null);
  let abortController: AbortController | null = null;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let stopped = false;

  const tick = async () => {
    if (stopped) return;
    abortController?.abort();
    const ac = new AbortController();
    abortController = ac;
    try {
      const list = await getDevices(ac.signal);
      if (stopped) return;
      devices.value = list;
      error.value = null;
    } catch (e) {
      if (stopped || (e as Error).name === 'AbortError') return;
      error.value = (e as Error).message;
    } finally {
      if (!stopped) loading.value = false;
    }
  };

  const scheduleNext = () => {
    if (stopped) return;
    timer = setTimeout(async () => {
      await tick();
      scheduleNext();
    }, DEVICE_POLL_INTERVAL_MS);
  };

  const refresh = () => {
    void tick();
  };

  onMounted(() => {
    stopped = false;
    void tick();
    scheduleNext();
  });

  onBeforeUnmount(() => {
    stopped = true;
    abortController?.abort();
    if (timer) clearTimeout(timer);
  });

  return { devices, loading, error, refresh };
}
