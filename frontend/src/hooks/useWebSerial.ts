// Web Serial API 封装：连接 ESP32 串口、发送配网指令、等待 OK / REBOOT 回执

import { useCallback, useRef, useState } from 'react';
import { SERIAL_BAUD_RATE, SERIAL_RESPONSE_TIMEOUT_MS } from '../lib/constants';

export interface SerialConfigPayload {
  ssid: string;
  password: string;
  server: string; // host:port
  token: string;
}

export interface UseWebSerialResult {
  supported: boolean;
  connecting: boolean;
  error: string | null;
  log: string[];
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
  const [connecting, setConnecting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [log, setLog] = useState<string[]>([]);
  const portRef = useRef<SerialPortLike | null>(null);
  const readerRef = useRef<ReadableStreamDefaultReader<Uint8Array> | null>(null);
  const writerRef = useRef<WritableStreamDefaultWriter<Uint8Array> | null>(null);

  const pushLog = useCallback((line: string) => {
    const ts = new Date().toLocaleTimeString('zh-CN', { hour12: false });
    setLog((prev) => [...prev.slice(-32), `[${ts}] ${line}`]);
  }, []);

  const closeReader = useCallback(async () => {
    try {
      await readerRef.current?.cancel();
    } catch {
      // ignore
    }
    try {
      await readerRef.current?.releaseLock();
    } catch {
      // ignore
    }
    readerRef.current = null;
    try {
      await writerRef.current?.close();
    } catch {
      // ignore
    }
    try {
      await writerRef.current?.releaseLock();
    } catch {
      // ignore
    }
    writerRef.current = null;
  }, []);

  const disconnect = useCallback(async () => {
    await closeReader();
    try {
      await portRef.current?.close();
    } catch {
      // ignore
    }
    portRef.current = null;
    pushLog('串口已断开');
  }, [closeReader, pushLog]);

  const configure = useCallback(
    async (payload: SerialConfigPayload): Promise<{ ok: boolean; message: string }> => {
      if (!supported) {
        const msg = '当前浏览器不支持 Web Serial API（请使用 Chrome/Edge 89+）';
        setError(msg);
        return { ok: false, message: msg };
      }
      setConnecting(true);
      setError(null);
      setLog([]);
      try {
        // 1. 请求选择串口
        const port = await navigatorWithSerial.serial!.requestPort();
        portRef.current = port;
        await port.open({ baudRate: SERIAL_BAUD_RATE });
        pushLog(`已打开串口 @ ${SERIAL_BAUD_RATE} baud`);

        // 2. 启动读取循环（异步收集响应）
        const responsePromise = readUntilReboot(port);

        // 3. 发送配网命令
        if (!port.writable) throw new Error('串口不可写');
        const writer = port.writable.getWriter();
        writerRef.current = writer;
        const cmd = `CONFIG|ssid=${payload.ssid}|password=${payload.password}|server=${payload.server}|token=${payload.token}\n`;
        const data = new TextEncoder().encode(cmd);
        await writer.write(data);
        pushLog(`已发送：${cmd.trim()}`);
        await writer.close();
        writerRef.current = null;

        // 4. 等待响应（OK / REBOOT / ERR）
        const result = await responsePromise;
        await disconnect();
        return result;
      } catch (e) {
        const msg = (e as Error).message ?? String(e);
        setError(msg);
        pushLog(`错误：${msg}`);
        await disconnect();
        return { ok: false, message: msg };
      } finally {
        setConnecting(false);
      }
    },
    [supported, pushLog, disconnect, navigatorWithSerial],
  );

  /** 读取串口直到收到 REBOOT 或 ERR 或超时 */
  const readUntilReboot = useCallback(
    async (port: SerialPortLike): Promise<{ ok: boolean; message: string }> => {
      if (!port.readable) throw new Error('串口不可读');
      const reader = port.readable.getReader();
      readerRef.current = reader;
      const decoder = new TextDecoder();
      let acc = '';
      const start = Date.now();

      return new Promise<{ ok: boolean; message: string }>((resolve) => {
        const timeout = setTimeout(() => {
          resolve({ ok: false, message: `等待响应超时（${SERIAL_RESPONSE_TIMEOUT_MS}ms）` });
        }, SERIAL_RESPONSE_TIMEOUT_MS);

        const pump = async (): Promise<void> => {
          try {
            while (true) {
              if (Date.now() - start > SERIAL_RESPONSE_TIMEOUT_MS) {
                clearTimeout(timeout);
                resolve({ ok: false, message: '等待响应超时' });
                return;
              }
              const { done, value } = await reader.read();
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
    },
    [pushLog],
  );

  return { supported, connecting, error, log, configure, disconnect };
}
