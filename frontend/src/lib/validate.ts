// deviceId 格式校验 — 防止 DeviceSelect option textContent 干扰 value 提取

const DEVICE_ID_RE = /^ESP32S3_[0-9A-Fa-f]{6,}_[0-9A-Fa-f]{6,}$/;

/** 校验 deviceId 是否合法（ESP32S3_<chipId>_<mac>，hex 长度 >=6） */
export function isValidDeviceId(id: string): boolean {
  return DEVICE_ID_RE.test(id);
}
