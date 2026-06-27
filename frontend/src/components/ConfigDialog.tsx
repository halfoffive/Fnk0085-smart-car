// Web Serial 配网弹窗：表单 → 选串口 → 发送 CONFIG → 等 OK / REBOOT

import { FormEvent, useEffect, useState } from 'react';
import { useWebSerial } from '../hooks/useWebSerial';

interface ConfigDialogProps {
  open: boolean;
  onClose: () => void;
}

const DEFAULT_FORM = {
  ssid: '',
  password: '',
  server: '',
  token: '',
};

export function ConfigDialog({ open, onClose }: ConfigDialogProps) {
  const serial = useWebSerial();
  const [form, setForm] = useState(DEFAULT_FORM);
  const [resultMsg, setResultMsg] = useState<string | null>(null);
  const [resultOk, setResultOk] = useState<boolean | null>(null);

  // 关闭时复位
  useEffect(() => {
    if (!open) {
      setResultMsg(null);
      setResultOk(null);
    }
  }, [open]);

  // ESC 关闭
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && !serial.connecting) onClose();
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, serial.connecting, onClose]);

  if (!open) return null;

  const handleSubmit = async (e: FormEvent) => {
    e.preventDefault();
    if (!form.ssid || !form.server || !form.token) {
      setResultOk(false);
      setResultMsg('请完整填写 SSID / 服务器地址 / token');
      return;
    }
    setResultMsg(null);
    const result = await serial.configure(form);
    setResultOk(result.ok);
    setResultMsg(result.message);
  };

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 backdrop-blur-sm"
      onClick={(e) => {
        if (e.target === e.currentTarget && !serial.connecting) onClose();
      }}
    >
      <div className="relative w-full max-w-lg mx-4 bg-surface border border-border-bright shadow-2xl">
        {/* 顶部条 */}
        <div className="flex items-center justify-between border-b border-border px-4 py-3">
          <div className="flex items-center gap-2">
            <span className="w-1 h-4 bg-accent" />
            <h2 className="font-mono text-xs uppercase tracking-[0.3em] text-ink">
              web serial · provisioning
            </h2>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={serial.connecting}
            className="text-ink-faint hover:text-ink disabled:opacity-30 transition-colors"
            aria-label="关闭"
          >
            ✕
          </button>
        </div>

        {/* 兼容性提示 */}
        {!serial.supported && (
          <div className="px-4 py-2 bg-danger/10 border-b border-danger/30 font-mono text-[10px] text-danger">
            ⚠ 当前浏览器不支持 Web Serial API，请使用 Chrome/Edge 89+
          </div>
        )}

        <form onSubmit={handleSubmit} className="p-4 space-y-3">
          <Field
            label="WiFi SSID"
            value={form.ssid}
            onChange={(v) => setForm({ ...form, ssid: v })}
            placeholder="your-network-name"
            disabled={serial.connecting}
            autoFocus
          />
          <Field
            label="WiFi 密码"
            value={form.password}
            onChange={(v) => setForm({ ...form, password: v })}
            placeholder="••••••••"
            type="password"
            disabled={serial.connecting}
          />
          <Field
            label="服务器地址 (host:port)"
            value={form.server}
            onChange={(v) => setForm({ ...form, server: v })}
            placeholder="api.example.com:7000"
            disabled={serial.connecting}
          />
          <Field
            label="设备 token"
            value={form.token}
            onChange={(v) => setForm({ ...form, token: v })}
            placeholder="Bearer ..."
            disabled={serial.connecting}
            mono
          />

          {/* 结果提示 */}
          {resultMsg && (
            <div
              className={`px-3 py-2 border-l-2 font-mono text-[11px] ${
                resultOk
                  ? 'bg-accent/10 border-accent text-accent'
                  : 'bg-danger/10 border-danger text-danger'
              }`}
            >
              {resultMsg}
            </div>
          )}

          {/* 日志区 */}
          {serial.log.length > 0 && (
            <div className="bg-black/60 border border-border p-2 max-h-32 overflow-y-auto font-mono text-[10px] text-ink-dim space-y-0.5">
              {serial.log.map((line, i) => (
                <div key={i} className="leading-tight">
                  {line}
                </div>
              ))}
            </div>
          )}

          <div className="flex items-center justify-between pt-2">
            <div className="font-mono text-[9px] uppercase tracking-wider text-ink-faint">
              {serial.connecting ? 'working…' : 'ready'}
            </div>
            <div className="flex items-center gap-2">
              <button
                type="button"
                onClick={onClose}
                disabled={serial.connecting}
                className="px-3 py-2 font-mono text-[10px] uppercase tracking-wider text-ink-dim hover:text-ink border border-border-bright hover:border-ink-faint disabled:opacity-30 transition-colors"
              >
                cancel
              </button>
              <button
                type="submit"
                disabled={serial.connecting || !serial.supported}
                className="px-4 py-2 font-mono text-[10px] uppercase tracking-wider bg-accent text-black hover:bg-accent-dim disabled:opacity-40 disabled:cursor-not-allowed transition-colors"
              >
                {serial.connecting ? '⟳ provisioning…' : '⚡ connect & send'}
              </button>
            </div>
          </div>
        </form>
      </div>
    </div>
  );
}

interface FieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  type?: 'text' | 'password';
  disabled?: boolean;
  mono?: boolean;
  autoFocus?: boolean;
}

function Field({
  label,
  value,
  onChange,
  placeholder,
  type = 'text',
  disabled,
  mono,
  autoFocus,
}: FieldProps) {
  return (
    <label className="block">
      <div className="font-mono text-[9px] uppercase tracking-[0.2em] text-ink-faint mb-1">
        {label}
      </div>
      <input
        type={type}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={placeholder}
        disabled={disabled}
        autoFocus={autoFocus}
        className={`w-full bg-surface-2 border border-border-bright focus:border-accent focus:outline-none px-3 py-2 text-sm text-ink placeholder:text-ink-faint/50 disabled:opacity-50 transition-colors ${
          mono ? 'font-mono' : 'font-sans'
        }`}
        autoComplete="off"
        spellCheck={false}
      />
    </label>
  );
}
