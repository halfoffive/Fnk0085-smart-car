// WASD 键盘 composable：监听 keydown/keyup，触发方向回调；上下箭头调速。

import { onBeforeUnmount, onMounted, ref } from 'vue';
import type { Direction } from '../types/protocol';

const KEY_MAP: Record<string, Direction> = {
  KeyW: 'W',
  KeyA: 'A',
  KeyS: 'S',
  KeyD: 'D',
  ArrowLeft: 'A',
  ArrowRight: 'D',
};

export interface UseKeyboardOptions {
  /** 某方向被按下（首次，非重复触发） */
  onPress: (dir: Direction) => void;
  /** 某方向被释放 */
  onRelease: (dir: Direction) => void;
  /** 速度增减（slider 单位，如 +5 / -5） */
  onSpeedDelta?: (delta: number) => void;
  /** 是否启用（响应式 getter） */
  enabled?: () => boolean;
}

/** 当前被按下的方向集合响应式引用 */
export function useKeyboard(opts: UseKeyboardOptions) {
  const { onPress, onRelease, onSpeedDelta, enabled = () => true } = opts;
  const pressed = ref<Set<Direction>>(new Set());

  const isInputFocused = (e: KeyboardEvent) =>
    e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement;

  const handleDown = (e: KeyboardEvent) => {
    if (!enabled()) return;
    if (e.repeat) return;
    if (isInputFocused(e)) return;

    // 速度键：上下箭头
    if (e.code === 'ArrowUp') {
      e.preventDefault();
      onSpeedDelta?.(+5);
      return;
    }
    if (e.code === 'ArrowDown') {
      e.preventDefault();
      onSpeedDelta?.(-5);
      return;
    }

    const dir = KEY_MAP[e.code];
    if (!dir) return;
    e.preventDefault();
    if (pressed.value.has(dir)) return;
    const next = new Set(pressed.value);
    next.add(dir);
    pressed.value = next;
    onPress(dir);
  };

  const handleUp = (e: KeyboardEvent) => {
    const dir = KEY_MAP[e.code];
    if (!dir) return;
    if (!pressed.value.has(dir)) return;
    const next = new Set(pressed.value);
    next.delete(dir);
    pressed.value = next;
    // 只要还有其他方向键被按住，就不发送 stop
    if (next.size === 0) {
      onRelease(dir);
    }
  };

  const handleBlur = () => {
    pressed.value.forEach((d) => onRelease(d));
    pressed.value = new Set();
  };

  onMounted(() => {
    window.addEventListener('keydown', handleDown);
    window.addEventListener('keyup', handleUp);
    window.addEventListener('blur', handleBlur);
  });

  onBeforeUnmount(() => {
    window.removeEventListener('keydown', handleDown);
    window.removeEventListener('keyup', handleUp);
    window.removeEventListener('blur', handleBlur);
  });

  return { pressed };
}
