// WASD 键盘 hook：监听 keydown/keyup，触发回调（仅当方向键被按下/释放时调用）

import { useEffect, useRef } from 'react';
import type { Direction } from '../types/protocol';

const KEY_MAP: Record<string, Direction> = {
  KeyW: 'W',
  KeyA: 'A',
  KeyS: 'S',
  KeyD: 'D',
  ArrowUp: 'W',
  ArrowLeft: 'A',
  ArrowDown: 'S',
  ArrowRight: 'D',
};

export interface UseKeyboardOptions {
  /** 某方向被按下（首次，非重复触发） */
  onPress: (dir: Direction) => void;
  /** 某方向被释放 */
  onRelease: (dir: Direction) => void;
  /** 是否启用（默认 true） */
  enabled?: boolean;
}

export function useKeyboard(opts: UseKeyboardOptions) {
  const { onPress, onRelease, enabled = true } = opts;
  const pressedRef = useRef<Set<Direction>>(new Set());
  const cbRef = useRef({ onPress, onRelease });
  cbRef.current = { onPress, onRelease };

  useEffect(() => {
    if (!enabled) return;
    const handleDown = (e: KeyboardEvent) => {
      if (e.repeat) return;
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      const dir = KEY_MAP[e.code];
      if (!dir) return;
      e.preventDefault();
      if (pressedRef.current.has(dir)) return;
      pressedRef.current.add(dir);
      cbRef.current.onPress(dir);
    };
    const handleUp = (e: KeyboardEvent) => {
      const dir = KEY_MAP[e.code];
      if (!dir) return;
      if (!pressedRef.current.has(dir)) return;
      pressedRef.current.delete(dir);
      cbRef.current.onRelease(dir);
    };
    const handleBlur = () => {
      // 失焦时释放所有按键，避免悬挂
      pressedRef.current.forEach((d) => cbRef.current.onRelease(d));
      pressedRef.current.clear();
    };
    window.addEventListener('keydown', handleDown);
    window.addEventListener('keyup', handleUp);
    window.addEventListener('blur', handleBlur);
    return () => {
      window.removeEventListener('keydown', handleDown);
      window.removeEventListener('keyup', handleUp);
      window.removeEventListener('blur', handleBlur);
    };
  }, [enabled]);

  /** 当前被按下的方向集合 */
  const pressed = pressedRef.current;
  return { pressed };
}
