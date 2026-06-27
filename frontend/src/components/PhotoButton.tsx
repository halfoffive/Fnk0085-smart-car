// 拍照按钮：点击 → 暂停视频流 → POST /api/photo → 等待回执 → 恢复视频流 → toast

import { useState } from 'react';
import { postPhoto } from '../lib/api';

interface PhotoButtonProps {
  deviceId: string | null;
  /** 拍照期间外部暂停视频流的回调 */
  onBeforeCapture: () => void;
  onAfterCapture: () => void;
  /** 拍照完成回调 */
  onResult: (path: string) => void;
  onError: (msg: string) => void;
}

export function PhotoButton({
  deviceId,
  onBeforeCapture,
  onAfterCapture,
  onResult,
  onError,
}: PhotoButtonProps) {
  const [loading, setLoading] = useState(false);
  const disabled = !deviceId || loading;

  const handleClick = async () => {
    if (!deviceId || loading) return;
    setLoading(true);
    onBeforeCapture();
    try {
      const res = await postPhoto(deviceId);
      onResult(res.path);
    } catch (e) {
      onError((e as Error).message);
    } finally {
      setLoading(false);
      onAfterCapture();
    }
  };

  return (
    <button
      type="button"
      onClick={handleClick}
      disabled={disabled}
      className="group relative w-full overflow-hidden bg-surface-2 border border-amber/40 hover:border-amber focus:outline-none focus:ring-1 focus:ring-amber disabled:opacity-40 disabled:cursor-not-allowed transition-all"
    >
      {/* 加载圈 */}
      {loading ? (
        <div className="flex items-center justify-center gap-3 py-3.5">
          <div className="w-4 h-4 conic-spinner rounded-full" />
          <span className="font-mono text-xs uppercase tracking-[0.3em] text-amber">
            capturing…
          </span>
        </div>
      ) : (
        <div className="flex items-center justify-center gap-3 py-3.5">
          {/* 相机图标 */}
          <svg width="16" height="14" viewBox="0 0 16 14" fill="none" className="text-amber">
            <rect
              x="0.75"
              y="2.75"
              width="14.5"
              height="9.5"
              rx="1.25"
              stroke="currentColor"
              strokeWidth="1.5"
            />
            <path d="M5 2.75 L5.5 1.5 L10.5 1.5 L11 2.75" stroke="currentColor" strokeWidth="1.5" />
            <circle cx="8" cy="7.5" r="2.5" stroke="currentColor" strokeWidth="1.5" />
          </svg>
          <span className="font-mono text-xs uppercase tracking-[0.3em] text-ink group-hover:text-amber transition-colors">
            capture photo
          </span>
        </div>
      )}
      {/* 悬停光带 */}
      {!loading && (
        <div className="absolute inset-0 bg-gradient-to-r from-transparent via-amber/5 to-transparent -translate-x-full group-hover:translate-x-full transition-transform duration-700 ease-out" />
      )}
    </button>
  );
}
