// 控制面板：WASD D-pad + 1-100 调速滑块 + PWM 缓存开关 + 拍照按钮 + 配网入口

import { useCallback, useEffect, useRef, useState } from 'react';
import { useKeyboard } from '../hooks/useKeyboard';
import { getPwmCache, postControl, postPwmCache, sliderToPwm } from '../lib/api';
import { PWM_DEBOUNCE_MS, PWM_MAX, SLIDER_MAX, SLIDER_MIN } from '../lib/constants';
import type { Direction } from '../types/protocol';
import { PhotoButton } from './PhotoButton';

interface ControlPanelProps {
  deviceId: string | null;
  /** 当前 PWM 值（受控） */
  pwm: number;
  onPwmChange: (pwm: number) => void;
  /** WASD 状态变化回调（用于在父组件高亮显示，可选） */
  onActiveDirectionChange?: (dir: Direction | null) => void;
  onOpenConfig: () => void;
  onPhotoBefore: () => void;
  onPhotoAfter: () => void;
  onPhotoResult: (path: string) => void;
  onPhotoError: (msg: string) => void;
}

const DIR_META: Record<Direction, { label: string; sub: string }> = {
  W: { label: 'W', sub: 'fwd' },
  A: { label: 'A', sub: 'left' },
  S: { label: 'S', sub: 'back' },
  D: { label: 'D', sub: 'right' },
  stop: { label: '■', sub: 'stop' },
};

export function ControlPanel({
  deviceId,
  pwm,
  onPwmChange,
  onActiveDirectionChange,
  onOpenConfig,
  onPhotoBefore,
  onPhotoAfter,
  onPhotoResult,
  onPhotoError,
}: ControlPanelProps) {
  const [activeDir, setActiveDir] = useState<Direction | null>(null);
  const [cacheEnabled, setCacheEnabled] = useState<boolean | null>(null);
  const [cacheLoading, setCacheLoading] = useState(false);
  const inFlightRef = useRef<Set<Direction>>(new Set());
  const lastPwmRef = useRef<number>(pwm);
  const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  // 拉取 PWM 缓存状态
  useEffect(() => {
    if (!deviceId) {
      setCacheEnabled(null);
      return;
    }
    let cancelled = false;
    setCacheLoading(true);
    getPwmCache(deviceId)
      .then((s) => {
        if (!cancelled) setCacheEnabled(s.enabled);
      })
      .catch(() => {
        if (!cancelled) setCacheEnabled(false);
      })
      .finally(() => {
        if (!cancelled) setCacheLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, [deviceId]);

  // 同步最新 pwm 到 ref（供 keyboard 回调读取）
  useEffect(() => {
    lastPwmRef.current = pwm;
  }, [pwm]);

  // WASD 键盘控制
  const handlePress = useCallback(
    async (dir: Direction) => {
      if (!deviceId) return;
      if (inFlightRef.current.has(dir)) return;
      inFlightRef.current.add(dir);
      setActiveDir(dir);
      onActiveDirectionChange?.(dir);
      try {
        await postControl(deviceId, { direction: dir, pwm: lastPwmRef.current });
      } catch {
        // 静默：父组件 toast 由其它路径触发
      } finally {
        inFlightRef.current.delete(dir);
      }
    },
    [deviceId, onActiveDirectionChange],
  );

  const handleRelease = useCallback(
    async (dir: Direction) => {
      if (!deviceId) return;
      setActiveDir((curr) => (curr === dir ? null : curr));
      onActiveDirectionChange?.(null);
      try {
        await postControl(deviceId, { direction: 'stop', pwm: 0 });
      } catch {
        // ignore
      }
    },
    [deviceId, onActiveDirectionChange],
  );

  useKeyboard({
    onPress: handlePress,
    onRelease: handleRelease,
    enabled: !!deviceId,
  });

  // 点击 D-pad 按钮
  const handlePadDown = (dir: Direction) => {
    void handlePress(dir);
  };
  const handlePadUp = () => {
    void handleRelease(activeDir ?? 'stop');
  };

  // 滑块 debounce 下发
  const handleSliderChange = (val: number) => {
    const newPwm = sliderToPwm(val);
    onPwmChange(newPwm);
    if (debounceRef.current) clearTimeout(debounceRef.current);
    debounceRef.current = setTimeout(() => {
      // 滑动结束时不主动下发（WASD 时才发），仅同步显示
      // 这里仅做 UI 状态展示，实际下发由 WASD 按下时携带
    }, PWM_DEBOUNCE_MS);
  };

  // PWM 缓存开关
  const handleToggleCache = async () => {
    if (!deviceId || cacheEnabled === null || cacheLoading) return;
    const next = !cacheEnabled;
    setCacheLoading(true);
    setCacheEnabled(next); // 乐观更新
    try {
      await postPwmCache(deviceId, { enabled: next });
    } catch {
      setCacheEnabled(!next); // 回滚
    } finally {
      setCacheLoading(false);
    }
  };

  const sliderValue = Math.round((pwm / PWM_MAX) * 100);
  const pct = ((sliderValue - SLIDER_MIN) / (SLIDER_MAX - SLIDER_MIN)) * 100;

  return (
    <div className="flex flex-col h-full overflow-y-auto">
      {/* ============ WASD D-Pad ============ */}
      <Section title="drive ▸ wasd" hint="键盘 / 触控">
        <div className="grid grid-cols-3 grid-rows-2 gap-1.5 max-w-[220px] mx-auto select-none">
          {/* W */}
          <PadButton
            dir="W"
            active={activeDir === 'W'}
            onDown={handlePadDown}
            onUp={handlePadUp}
            className="col-start-2"
          />
          {/* A */}
          <PadButton
            dir="A"
            active={activeDir === 'A'}
            onDown={handlePadDown}
            onUp={handlePadUp}
            className="col-start-1 row-start-2"
          />
          {/* S */}
          <PadButton
            dir="S"
            active={activeDir === 'S'}
            onDown={handlePadDown}
            onUp={handlePadUp}
            className="col-start-2 row-start-2"
          />
          {/* D */}
          <PadButton
            dir="D"
            active={activeDir === 'D'}
            onDown={handlePadDown}
            onUp={handlePadUp}
            className="col-start-3 row-start-2"
          />
        </div>

        {/* 状态行 */}
        <div className="flex items-center justify-between mt-2 px-1 font-mono text-[9px] uppercase tracking-wider">
          <span className="text-ink-faint">
            active:{' '}
            <span className={activeDir ? 'text-accent' : 'text-ink-faint'}>
              {activeDir ?? 'idle'}
            </span>
          </span>
          <span className="text-ink-faint">
            target:{' '}
            <span className="text-ink">{deviceId ?? '—'}</span>
          </span>
        </div>
      </Section>

      {/* ============ 调速滑块 ============ */}
      <Section title="throttle ▸ pwm" hint={`${pwm} / ${PWM_MAX}`}>
        <div className="space-y-2">
          <input
            type="range"
            min={SLIDER_MIN}
            max={SLIDER_MAX}
            step={1}
            value={sliderValue}
            onChange={(e) => handleSliderChange(Number(e.target.value))}
            className="range-track w-full"
            style={{ ['--pct' as string]: `${pct}%` }}
            disabled={!deviceId}
          />
          <div className="flex items-center justify-between font-mono text-[9px] uppercase tracking-wider">
            <span className="text-ink-faint">{SLIDER_MIN}</span>
            <span className="text-ink-dim">
              slider <span className="text-ink">{sliderValue}</span> · pwm{' '}
              <span className="text-accent">{pwm}</span>
            </span>
            <span className="text-ink-faint">{SLIDER_MAX}</span>
          </div>
          {/* PWM 条 */}
          <div className="relative h-1 bg-surface-2 overflow-hidden">
            <div
              className="absolute inset-y-0 left-0 bar-fill transition-[width] duration-150"
              style={{ width: `${(pwm / PWM_MAX) * 100}%` }}
            />
          </div>
        </div>
      </Section>

      {/* ============ 拍照 ============ */}
      <Section title="capture ▸ photo">
        <PhotoButton
          deviceId={deviceId}
          onBeforeCapture={onPhotoBefore}
          onAfterCapture={onPhotoAfter}
          onResult={onPhotoResult}
          onError={onPhotoError}
        />
      </Section>

      {/* ============ PWM 缓存开关 ============ */}
      <Section title="tuning ▸ pwm cache" hint={cacheEnabled === null ? '—' : cacheEnabled ? 'on' : 'off'}>
        <div className="flex items-center justify-between bg-surface-2 border border-border-bright px-3 py-2.5">
          <div className="flex flex-col">
            <span className="font-mono text-[11px] text-ink">PWM 缓存</span>
            <span className="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
              PID 收敛结果复用
            </span>
          </div>
          <button
            type="button"
            role="switch"
            aria-checked={cacheEnabled === true}
            onClick={handleToggleCache}
            disabled={!deviceId || cacheLoading || cacheEnabled === null}
            className="toggle-switch disabled:opacity-30 disabled:cursor-not-allowed"
            aria-label="切换 PWM 缓存"
          />
        </div>
        <div className="flex items-center gap-1 mt-1.5 font-mono text-[9px] uppercase tracking-wider text-ink-faint">
          <span>cache:</span>
          <span className={cacheEnabled ? 'text-accent' : 'text-ink-dim'}>
            {cacheLoading ? '⟳ updating' : cacheEnabled === null ? 'unknown' : cacheEnabled ? 'enabled' : 'disabled'}
          </span>
        </div>
      </Section>

      {/* ============ 配网入口 ============ */}
      <Section title="provision ▸ web serial">
        <button
            type="button"
            onClick={onOpenConfig}
            className="w-full group flex items-center justify-between bg-surface-2 border border-border-bright hover:border-accent/60 px-3 py-2.5 transition-colors"
          >
          <div className="flex flex-col items-start">
            <span className="font-mono text-[11px] text-ink group-hover:text-accent transition-colors">
              打开配网弹窗
            </span>
            <span className="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
              WiFi · server · token
            </span>
          </div>
          <svg width="14" height="14" viewBox="0 0 14 14" fill="none" className="text-ink-faint group-hover:text-accent transition-colors">
            <path d="M5 3 L9 7 L5 11" stroke="currentColor" strokeWidth="1.5" />
          </svg>
        </button>
      </Section>
    </div>
  );
}

// ============ 子组件 ============

interface PadButtonProps {
  dir: Direction;
  active: boolean;
  onDown: (dir: Direction) => void;
  onUp: () => void;
  className?: string;
}

function PadButton({ dir, active, onDown, onUp, className = '' }: PadButtonProps) {
  const meta = DIR_META[dir];
  return (
    <button
      type="button"
      onPointerDown={(e) => {
        e.preventDefault();
        onDown(dir);
      }}
      onPointerUp={(e) => {
        e.preventDefault();
        onUp();
      }}
      onPointerLeave={() => {
        if (active) onUp();
      }}
      onPointerCancel={onUp}
      className={`relative flex flex-col items-center justify-center aspect-square bg-surface-2 border transition-all ${
        active
          ? 'border-accent bg-accent/10 key-active scale-95'
          : 'border-border-bright hover:border-accent/40'
      } ${className}`}
    >
      <span
        className={`font-mono text-lg font-bold ${
          active ? 'text-accent' : 'text-ink'
        }`}
      >
        {meta.label}
      </span>
      <span className="font-mono text-[8px] uppercase tracking-wider text-ink-faint">
        {meta.sub}
      </span>
      {active && (
        <span className="absolute top-1 right-1 w-1 h-1 bg-accent rounded-full" />
      )}
    </button>
  );
}

interface SectionProps {
  title: string;
  hint?: string;
  children: React.ReactNode;
}

function Section({ title, hint, children }: SectionProps) {
  return (
    <section className="px-4 py-3 border-b border-border">
      <div className="flex items-baseline justify-between mb-2">
        <h3 className="font-mono text-[9px] uppercase tracking-[0.25em] text-ink-faint">
          {title}
        </h3>
        {hint && (
          <span className="font-mono text-[9px] uppercase tracking-wider text-ink-faint/70">
            {hint}
          </span>
        )}
      </div>
      {children}
    </section>
  );
}
