# Fnk0085 智能小车通信协议 v1

> 开发参考协议文档（非用户文档）。锁定设备 ↔ 后端 ↔ 前端的通信契约。
> 任何字段变更必须同步更新本文档与三方实现（firmware/.ino、backend/、frontend/）。

---

## 1. UDP 视频分包包格式（设备 → 后端）

通过 DTLS（UDP+TLS）发送，端口由后端配置决定（默认 7000）。
所有多字节整数采用**小端序（little-endian）**。

### 1.1 字节布局

| 偏移 | 字段 | 长度 | 类型 | 说明 |
|------|------|------|------|------|
| 0 | magic | 2B | uint8[2] | 固定 `0xF1 0xD0`，用于包识别 |
| 2 | version | 1B | uint8 | 协议版本，初始 `1` |
| 3 | deviceIdLen | 1B | uint8 | deviceId 字符串长度 N |
| 4 | deviceId | N B | UTF-8 | 设备 ID，如 `ESP32S3_<chipId>_<MAC>` |
| 4+N | uptimeMs | 8B | uint64 LE | 设备系统运行时间（毫秒） |
| 12+N | frameSeq | 4B | uint32 LE | 帧序号（同一帧的所有 8 个分包相同） |
| 16+N | partIdx | 1B | uint8 | `0..7`，当前子包索引 |
| 17+N | partTotal | 1B | uint8 | 固定 `8` |
| 18+N | row | 1B | uint8 | `0..1`（2 行网格） |
| 19+N | col | 1B | uint8 | `0..3`（4 列网格） |
| 20+N | payloadLen | 4B | uint32 LE | 本子包 JPEG 切片字节长度 M |
| 24+N | payload | M B | bytes | JPEG 切片字节 |

### 1.2 切片语义说明

- 整帧 QVGA 320×240，按 2 行 × 4 列网格概念切片，每子包对应约 160×60 像素区域。
- **实际实现**：将整帧 JPEG 字节流按字节均分为 8 段，`row`/`col` 仅作为位置语义标记用于诊断与乱序检测，**不参与实际切分逻辑**。
- `payloadLen` 为本段实际字节数（最后一段可能小于平均长度）。
- 后端重组时按 `partIdx` 顺序拼接即可还原完整 JPEG。

---

## 2. 后端 → 设备控制指令（DTLS 下行）

JSON 格式（UTF-8）。

### 2.1 公共字段

| 字段 | 类型 | 说明 |
|------|------|------|
| type | string | `"control"` \| `"photo"` \| `"pwm_cache"` \| `"ping"` |
| ts | int | 后端发送时间戳（ms） |

### 2.2 type=control

```json
{
  "type": "control",
  "direction": "W",
  "pwm": 200,
  "durationMs": 500,
  "ts": 1719360000000
}
```

- `direction`: `"W"`（前进） \| `"A"`（左转） \| `"S"`（后退） \| `"D"`（右转） \| `"stop"`（停止）
- `pwm`: `0..255`
- `durationMs`: 可选，执行时长（毫秒），超时自动停止

### 2.3 type=photo

```json
{
  "type": "photo",
  "quality": "max",
  "ts": 1719360000000
}
```

- `quality`: 固定 `"max"`（仅一种质量档位）

### 2.4 type=pwm_cache

```json
{
  "type": "pwm_cache",
  "enabled": true,
  "ts": 1719360000000
}
```

- `enabled`: bool，是否启用 PWM 缓存

### 2.5 type=ping

```json
{
  "type": "ping",
  "seq": 42,
  "ts": 1719360000000
}
```

- `seq`: int，递增序号，用于往返时延测量

---

## 3. 设备 → 后端事件回执（DTLS 上行）

JSON 格式（UTF-8）。

### 3.1 公共字段

| 字段 | 类型 | 说明 |
|------|------|------|
| type | string | `"register"` \| `"photo_done"` \| `"ack"` \| `"error"` |

### 3.2 type=register

```json
{
  "type": "register",
  "deviceId": "ESP32S3_123456_AABBCCDDEEFF",
  "token": "Bearer xxxx"
}
```

初次认证时发送，后端校验 `token` 与 `deviceId` 绑定关系。

### 3.3 type=photo_done

```json
{
  "type": "photo_done",
  "path": "/sd/photo/001.jpg",
  "uptimeMs": 123456
}
```

拍照完成回执，`path` 为 SD 卡上的保存路径，`uptimeMs` 为设备系统运行时间（ms）。

### 3.4 type=ack

```json
{
  "type": "ack",
  "refSeq": 42
}
```

- `refSeq`: 对应 `ping` 的 `seq` 或 `control` 的隐式序号

### 3.5 type=error

```json
{
  "type": "error",
  "code": 5001,
  "message": "camera init failed"
}
```

- `code`: int，业务错误码
- `message`: string，可读错误描述

---

## 4. HTTP API（前端 ↔ 后端）

所有路径前缀 `/api`。响应默认 JSON（除流式接口）。

### 4.1 GET /api/devices

返回设备列表：

```json
[
  {"deviceId": "ESP32S3_123456_AABBCCDDEEFF", "online": true, "lastSeenMs": 1719360000000}
]
```

### 4.2 POST /api/control/{deviceId}

请求体：

```json
{"direction": "W", "pwm": 200}
```

- `direction`: `"W"` \| `"A"` \| `"S"` \| `"D"` \| `"stop"`
- `pwm`: `0..255`

响应：

```json
{"ok": true}
```

### 4.3 POST /api/photo/{deviceId}

同步等待设备 `photo_done` 后返回：

```json
{"path": "/sd/photo/001.jpg"}
```

### 4.4 GET /api/stream/{deviceId}

SSE 或 chunked transfer。每条消息为一帧完整 JPEG：

- Content-Type: `image/jpeg`（每帧）
- 响应头 `X-Latency-Ms: <int>`：本帧端到端延迟（ms）

### 4.5 GET /api/pwm_cache/{deviceId}

```json
{
  "enabled": true,
  "entries": [{"speed": "low", "pwm": 120}, {"speed": "high", "pwm": 255}]
}
```

### 4.6 POST /api/pwm_cache/{deviceId}

请求体：

```json
{"enabled": true}
```

响应：

```json
{"ok": true}
```

### 4.7 静态资源

- `GET /` → 返回 `index.html`（内嵌前端 PWA）

---

## 5. Web Serial 配网帧格式（前端 → ESP32 串口）

ASCII 行协议，每行一条命令，以 `\n` 结尾。

### 5.1 配网命令

```
CONFIG|ssid=<ssid>|password=<pwd>|server=<host:port>|token=<token>\n
```

字段说明：

- `ssid`: WiFi SSID
- `password`: WiFi 密码
- `server`: 后端 DTLS 主机:端口
- `token`: 设备认证 token

### 5.2 ESP32 回复

成功：

```
OK\n
```

随后写入 NVS 并执行：

```
REBOOT\n
```

失败：

```
ERR|<reason>\n
```

`reason` 为可读失败原因（如 `invalid_token`、`nvs_write_failed`）。

---

## 6. 鉴权

### 6.1 设备 → 后端 DTLS

- 双向证书（mTLS）认证
- `register` 帧中 `token` 与 `deviceId` 绑定校验
- 证书路径与 CA 由后端配置决定（`*.jsonc`）

### 6.2 前端 → 后端 HTTP

- 可选 Bearer token（如配置启用）
- 由后端配置决定是否强制鉴权

---

## 附录：版本与维护

- 协议版本：`1`（`version` 字段初始值）
- 任何字段变更需同步更新：`firmware/.ino`、`backend/`、`frontend/`、本文档
- 保留 magic `0xF1 0xD0` 用于未来版本兼容性识别
