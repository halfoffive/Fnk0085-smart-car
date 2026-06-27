// 主应用：左侧视频流 + 右侧控制面板 + 顶部状态条 + Toast + 配网弹窗 + PWA 注册

import { useCallback, useEffect, useRef, useState } from 'react';
import { DeviceSelect } from './components/DeviceSelect';
import { VideoStream } from './components/VideoStream';
import { ControlPanel } from './components/ControlPanel';
import { ConfigDialog } from './components/ConfigDialog';
import { useDevices } from './hooks/useDevices';
import { sliderToPwm } from './lib/api';
import { APP_VERSION, PWM_MAX } from './lib/constants';
import { registerSW } from 'virtual:pwa-register';

type ToastKind = 'info' | 'success' | 'error';
interface Toast {
  id: number;
  kind: ToastKind;
  message: string;
}

export default function App() {
  const { devices, loading, error, refresh } = useDevices();
  const [selectedDevice, setSelectedDevice] = useState<string | null>(null);
  const [pwm, setPwm] = useState<number>(sliderToPwm(50));
  const [configOpen, setConfigOpen] = useState(false);
  const [photoPending, setPhotoPending] = useState(false);
  const [toasts, setToasts] = useState<Toast[]>([]);
  const toastIdRef = useRef(0);

  // PWA SW 注册：版本更新提示
  useEffect(() => {
    if (import.meta.env.PROD) {
      registerSW({
        onRegistered: (reg) => {
          // eslint-disable-next-line no-console
          console.info('[PWA] service worker registered', reg?.scope);
        },
        onRegisterError: (err) => {
          console.warn('[PWA] sw register failed', err);
        },
      });
    }
  }, []);

  // 默认选第一个在线设备
  useEffect(() => {
    if (selectedDevice) {
      const stillExists = devices.some((d) => d.deviceId === selectedDevice);
      if (stillExists) return;
    }
    const firstOnline = devices.find((d) => d.online);
    if (firstOnline) {
      setSelectedDevice(firstOnline.deviceId);
    } else if (devices.length > 0) {
      setSelectedDevice(null);
    }
  }, [devices, selectedDevice]);

  const pushToast = useCallback(
    (kind: ToastKind, message: string) => {
      const id = ++toastIdRef.current;
      setToasts((prev) => [...prev, { id, kind, message }]);
      setTimeout(() => {
        setToasts((prev) => prev.filter((t) => t.id !== id));
      }, 4000);
    },
    [toastIdRef],
  );

  const handlePhotoResult = useCallback(
    (path: string) => {
      pushToast('success', `已拍照：${path}`);
    },
    [pushToast],
  );

  const handlePhotoError = useCallback(
    (msg: string) => {
      pushToast('error', `拍照失败：${msg}`);
    },
    [pushToast],
  );

  const onlineCount = devices.filter((d) => d.online).length;

  return (
    <div className="h-screen w-screen flex flex-col bg-base text-ink overflow-hidden">
      {/* ============ 顶部状态条 ============ */}
      <header className="flex items-center justify-between px-4 h-12 border-b border-border bg-surface/80 backdrop-blur z-30">
        {/* 左：品牌 */}
        <div className="flex items-center gap-3">
          <div className="flex items-center gap-2">
            <span className="w-2 h-2 bg-accent pulse-dot" />
            <span className="font-mono text-sm font-bold tracking-tight">
              Fnk<span className="text-accent">0085</span>
            </span>
          </div>
          <span className="font-mono text-[9px] uppercase tracking-[0.25em] text-ink-faint">
            smart car console
          </span>
        </div>

        {/* 中：导航 */}
        <nav className="flex items-center gap-4 font-mono text-[10px] uppercase tracking-wider text-ink-dim">
          <span className="flex items-center gap-1.5">
            <span className="w-1 h-1 bg-accent" />
            <span>console</span>
          </span>
          <span className="text-ink-faint">/</span>
          <span className="text-ink-faint">fleet</span>
          <span className="text-ink-faint">/</span>
          <span className="text-ink-faint">telemetry</span>
        </nav>

        {/* 右：状态 */}
        <div className="flex items-center gap-4 font-mono text-[10px] uppercase tracking-wider">
          <span className="flex items-center gap-1.5">
            <span className="text-ink-faint">devices</span>
            <span className="text-ink">
              {onlineCount}/{devices.length}
            </span>
          </span>
          <span className="text-ink-faint">·</span>
          <span className="text-ink-dim">
            v<span className="text-ink">{APP_VERSION}</span>
          </span>
        </div>
      </header>

      {/* ============ 主区域 ============ */}
      <main className="flex-1 flex min-h-0">
        {/* 左侧：视频流 + 设备状态条 */}
        <div className="flex-1 flex flex-col min-w-0 border-r border-border">
          {/* 设备状态条 */}
          <div className="flex items-center gap-3 px-4 h-9 bg-surface-2 border-b border-border font-mono text-[10px] uppercase tracking-wider">
            <span className="text-ink-faint">device ▸</span>
            <span className="truncate-mono text-ink max-w-md">
              {selectedDevice ?? '— no device —'}
            </span>
            <div className="ml-auto flex items-center gap-3">
              <span className="text-ink-faint">
                online:{' '}
                <span className="text-accent">{onlineCount}</span>
              </span>
              <span className="text-ink-faint">·</span>
              <span className="text-ink-faint">
                pwm:{' '}
                <span className="text-amber">
                  {pwm}/{PWM_MAX}
                </span>
              </span>
            </div>
          </div>

          {/* 视频流 */}
          <div className="flex-1 min-h-0 relative">
            <VideoStream deviceId={selectedDevice} paused={false} photoPending={photoPending} />
          </div>
        </div>

        {/* 右侧：控制面板 */}
        <aside className="w-[340px] flex flex-col bg-surface min-h-0">
          {/* 设备选择 */}
          <div className="px-4 py-3 border-b border-border">
            <DeviceSelect
              devices={devices}
              selected={selectedDevice}
              loading={loading}
              error={error}
              onSelect={setSelectedDevice}
              onRefresh={refresh}
            />
          </div>

          {/* 控制面板 */}
          <div className="flex-1 min-h-0">
            <ControlPanel
              deviceId={selectedDevice}
              pwm={pwm}
              onPwmChange={setPwm}
              onOpenConfig={() => setConfigOpen(true)}
              onPhotoBefore={() => setPhotoPending(true)}
              onPhotoAfter={() => setPhotoPending(false)}
              onPhotoResult={handlePhotoResult}
              onPhotoError={handlePhotoError}
            />
          </div>

          {/* 底部信息条 */}
          <footer className="px-4 py-2 border-t border-border font-mono text-[8px] uppercase tracking-wider text-ink-faint flex items-center justify-between">
            <span>udp+tls · dtls · mjpeg</span>
            <span className="flex items-center gap-1">
              <span className="w-1 h-1 bg-accent rounded-full" />
              <span>system nominal</span>
            </span>
          </footer>
        </aside>
      </main>

      {/* ============ Toast ============ */}
      <div className="fixed top-14 right-4 z-50 flex flex-col gap-2 pointer-events-none">
        {toasts.map((t) => (
          <div
            key={t.id}
            className={`toast-in pointer-events-auto px-3 py-2 min-w-[240px] max-w-sm border-l-2 font-mono text-[11px] backdrop-blur ${
              t.kind === 'success'
                ? 'bg-accent/10 border-accent text-accent'
                : t.kind === 'error'
                  ? 'bg-danger/10 border-danger text-danger'
                  : 'bg-surface/90 border-border-bright text-ink'
            }`}
          >
            <div className="flex items-start gap-2">
              <span className="font-bold">
                {t.kind === 'success' ? '✓' : t.kind === 'error' ? '✕' : 'ℹ'}
              </span>
              <span className="flex-1 break-words">{t.message}</span>
            </div>
          </div>
        ))}
      </div>

      {/* ============ 配网弹窗 ============ */}
      <ConfigDialog open={configOpen} onClose={() => setConfigOpen(false)} />
    </div>
  );
}
