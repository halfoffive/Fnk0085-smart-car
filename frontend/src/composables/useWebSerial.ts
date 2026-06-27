// Web Serial API 封装：连接 ESP32 串口、发送配网指令、等待 OK / REBOOT 回执

import { onBeforeUnmount, ref, type Ref } from 'vue';
import { SERIAL_BAUD_RATE, SERIAL_RESPONSE_TIMEOUT_MS } from '../lib/constants';

export interface SerialConfigPayload {
  ssid: string;
  password: string;
  server: string; // http(s)://host:port
  token: string;
}

export interface UseWebSerialResult {
  supported: boolean;
  connecting: Ref<boolean>;
  error: Ref<string | null>;
  log: Ref<string[]>;
  /** 请求选择串口并发送配置 */
  configure: (payload: SerialConfigPayload) => Promise<{ ok: boolean; message: string }>;
  /** 主动断开 */
  disconnect: () => Promise<void>;
}

/** Web Serial 类型在 lib.dom 中存在性不稳定，这里做最小类型断言 */
interface SerialPortLike {
  open(opts: { baudRate: number }): Promise<void>;
  close(): Promise<void>;
  readable: ReadableStream<Uint8Array> | null;
  writable: WritableStream<Uint8Array> | null;
}
interface SerialLike {
  requestPort(opts?: unknown): Promise<SerialPortLike>;
}
interface NavigatorWithSerial extends Navigator {
  serial?: SerialLike;
}

export function useWebSerial(): UseWebSerialResult {
  const navigatorWithSerial = navigator as NavigatorWithSerial;
  const supported = typeof navigatorWithSerial.serial !== 'undefined';
  const connecting = ref(false);
  const error = ref<string | null>(null);
  const log = ref<string[]>([]);
  let port: SerialPortLike | null = null;
  let reader: ReadableStreamDefaultReader<Uint8Array> | null = null;
  let writer: WritableStreamDefaultWriter<Uint8Array> | null = null;

  const pushLog = (line: string) => {
    const ts = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    log.value = [...log.value.slice(-32), `[${ts}] ${line}`];
  };

  const closeReader = async () => {
    try {
      await reader?.cancel();
    } catch {
      // ignore
    }
    try {
      await reader?.releaseLock();
    } catch {
      // ignore
    }
    reader = null;
    try {
      await writer?.close();
    } catch {
      // ignore
    }
    try {
      await writer?.releaseLock();
    } catch {
      // ignore
    }
    writer = null;
  };

  const disconnect = async () => {
    await closeReader();
    try {
      await port?.close();
    } catch {
      // ignore
    }
    port = null;
    pushLog('串口已断开');
  };

  /** 读取串口直到收到 REBOOT 或 ERR 或超时 */
  const readUntilReboot = (p: SerialPortLike): Promise<{ ok: boolean; message: string }> => {
    if (!p.readable) throw new Error('串口不可读');
    reader = p.readable.getReader();
    const decoder = new TextDecoder();
    let acc = '';
    const start = Date.now();

    return new Promise<{ ok: boolean; message: string }>((resolve) => {
      const timeout = setTimeout(() => {
        resolve({ ok: false, message: `等待响应超时（${SERIAL_RESPONSE_TIMEOUT_MS}ms）` });
      }, SERIAL_RESPONSE_TIMEOUT_MS);

      const pump = async () => {
        try {
          while (true) {
            if (Date.now() - start > SERIAL_RESPONSE_TIMEOUT_MS) {
              clearTimeout(timeout);
              resolve({ ok: false, message: '等待响应超时' });
              return;
            }
            const { done, value } = await reader!.read();
            if (done) {
              clearTimeout(timeout);
              resolve({ ok: false, message: '串口在响应前关闭' });
              return;
            }
            const chunk = decoder.decode(value, { stream: true });
            acc += chunk;
            // 按行处理
            const lines = acc.split('\n');
            acc = lines.pop() ?? '';
            for (const raw of lines) {
              const line = raw.trim();
              if (!line) continue;
              pushLog(`← ${line}`);
              if (line === 'OK') {
                // 继续等待 REBOOT
              } else if (line === 'REBOOT') {
                clearTimeout(timeout);
                resolve({ ok: true, message: '配网成功，设备重启中…' });
                return;
              } else if (line.startsWith('ERR|')) {
                clearTimeout(timeout);
                const reason = line.slice(4);
                resolve({ ok: false, message: `配网失败：${reason}` });
                return;
              }
            }
          }
        } catch (e) {
          clearTimeout(timeout);
          resolve({ ok: false, message: `读取失败：${(e as Error).message}` });
        }
      };
      void pump();
    });
  };

  const configure = async (
    payload: SerialConfigPayload,
  ): Promise<{ ok: boolean; message: string }> => {
    if (!supported) {
      const msg = '当前浏览器不支持 Web Serial API（请使用 Chrome/Edge 89+）';
      error.value = msg;
      return { ok: false, message: msg };
    }
    connecting.value = true;
    error.value = null;
    log.value = [];
    try {
      // 1. 请求选择串口
      const p = await navigatorWithSerial.serial!.requestPort();
      port = p;
      await p.open({ baudRate: SERIAL_BAUD_RATE });
      pushLog(`已打开串口 @ ${SERIAL_BAUD_RATE} baud`);

      // 2. 启动读取循环（异步收集响应）
      const responsePromise = readUntilReboot(p);

      // 3. 发送配网命令
      if (!p.writable) throw new Error('串口不可写');
      writer = p.writable.getWriter();
      const cmd = `CONFIG|ssid=${payload.ssid}|password=${payload.password}|server=${payload.server}|token=${payload.token}\n`;
      const data = new TextEncoder().encode(cmd);
      await writer.write(data);
      pushLog(`已发送：${cmd.trim()}`);
      await writer.close();
      writer = null;

      // 4. 等待响应（OK / REBOOT / ERR）
      const result = await responsePromise;
      await disconnect();
      return result;
    } catch (e) {
      const msg = (e as Error).message ?? String(e);
      error.value = msg;
      pushLog(`错误：${msg}`);
      await disconnect();
      return { ok: false, message: msg };
    } finally {
      connecting.value = false;
    }
  };

  onBeforeUnmount(() => {
    void disconnect();
  });

  return { supported, connecting, error, log, configure, disconnect };
}
