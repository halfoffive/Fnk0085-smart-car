# Fnk0085 智能小车通信协议 v1

> 开发参考协议文档（非用户文档）。锁定设备 ↔ 后端 ↔ 前端的通信契约。
> 任何字段变更必须同步更新本文档与三方实现（firmware/.ino、backend/、frontend/）。

---

## 1. UDP 视频分包包格式（设备 → 后端）

视频流走 UDP + 应用层 AES-128-GCM AEAD（Arduino-ESP32 无高层 DTLS API，用 AEAD 等价实现 DTLS 安全属性），端口由后端配置决定（默认 7000）。控制指令不走此通道，见第 2 节 HTTPS。
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

## 2. HTTPS 设备控制通道（设备 ↔ 后端）

控制平面走 HTTPS（HTTP/2 over TLS，同端口 8080）。设备端使用 `WiFiClientSecure::setInsecure()` 信任后端自签证书（开发期方案；生产应换 mTLS / CA pinning）。

### 2.1 端点

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/api/device/{deviceId}/register` | 设备注册：body `{"token":"Bearer xxx"}` |
| GET  | `/api/device/{deviceId}/poll?timeout=N` | 长轮询拉指令：返回指令 JSON 或 `{"type":"ping"}`（超时占位，N 默认 30，上限 60） |
| POST | `/api/device/{deviceId}/event` | 设备上报事件：`photo_done` / `ack` / `error` |

所有端点（除 register）需带 `Authorization: Bearer <token>` 头。register 端点用 body 中的 token 校验。

### 2.2 下行指令（poll 返回）

JSON 格式（UTF-8），tag=`type`，字段 camelCase。

#### 2.2.1 type=control

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

#### 2.2.2 type=photo

```json
{
  "type": "photo",
  "quality": "max",
  "ts": 1719360000000
}
```

- `quality`: 固定 `"max"`（仅一种质量档位）

#### 2.2.3 type=pwm_cache

```json
{
  "type": "pwm_cache",
  "enabled": true,
  "ts": 1719360000000
}
```

- `enabled`: bool，是否启用 PWM 缓存

#### 2.2.4 type=ping

```json
{
  "type": "ping",
  "seq": 42,
  "ts": 1719360000000
}
```

- `seq`: int，递增序号，用于往返时延测量

### 2.3 上行事件（POST event）

JSON 格式（UTF-8），tag=`type`，字段 camelCase。

#### 2.3.1 type=photo_done

```json
{
  "type": "photo_done",
  "path": "/photo/photo_123456.jpg",
  "uptimeMs": 123456
}
```

拍照完成回执，`path` 为 SD 卡上的保存路径，`uptimeMs` 为设备系统运行时间（ms）。

#### 2.3.2 type=ack

```json
{
  "type": "ack",
  "refSeq": 42
}
```

- `refSeq`: 对应 `ping` 的 `seq`

#### 2.3.3 type=error

```json
{
  "type": "error",
  "code": 5002,
  "message": "photo capture or sd write failed"
}
```

- `code`: int，业务错误码
- `message`: string，可读错误描述

> register 事件不接受在 `/event` 端点上报，必须走 `/register` 端点。

---

## 3. 浏览器 HTTP API（前端 ↔ 后端）

所有路径前缀 `/api`。响应默认 JSON（除流式接口）。

### 3.1 GET /api/devices

返回设备列表：

```json
[
  {"deviceId": "ESP32S3_123456_AABBCCDDEEFF", "online": true, "lastSeenMs": 1719360000000}
]
```

### 3.2 POST /api/control/{deviceId}

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

### 3.3 POST /api/photo/{deviceId}

同步等待设备 `photo_done`（8s 超时）后返回：

```json
{"path": "/photo/photo_001.jpg"}
```

### 3.4 GET /api/stream/{deviceId}

SSE 或 chunked transfer。每条消息为一帧完整 JPEG：

- Content-Type: `image/jpeg`（每帧）
- 响应头 `X-Latency-Ms: <int>`：本帧端到端延迟（ms）

### 3.5 GET /api/pwm_cache/{deviceId}

```json
{
  "enabled": true,
  "entries": [{"speed": "low", "pwm": 120}, {"speed": "high", "pwm": 255}]
}
```

### 3.6 POST /api/pwm_cache/{deviceId}

请求体：

```json
{"enabled": true}
```

响应：

```json
{"ok": true}
```

### 3.7 静态资源

- `GET /` → 返回 `index.html`（内嵌前端 PWA）

---

## 4. Web Serial 配网帧格式（前端 → ESP32 串口）

ASCII 行协议，每行一条命令，以 `\n` 结尾。

### 4.1 配网命令

```
CONFIG|ssid=<ssid>|password=<pwd>|server=<host:port>|token=<token>\n
```

字段说明：

- `ssid`: WiFi SSID
- `password`: WiFi 密码
- `server`: 后端 HTTPS 主机:端口（默认 8080，与浏览器访问同端口）
- `token`: 设备认证 token

### 4.2 查询命令（串口）

```
CONFIG\n
```

单独发送 `CONFIG` 行不带任何字段时，设备从 NVS 读出当前已存的 `ssid` / `password` 长度 / `server` / `token` 并在串口回显，用于排查配网问题。

### 4.3 ESP32 回复

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

## 5. 鉴权

### 5.1 设备 → 后端 HTTPS

- TLS 自签证书；设备端通过 `WiFiClientSecure::setInsecure()` 信任（开发期方案，跳过证书校验）
- 生产部署应改为 `setCACert(root_ca)` 或 mTLS 客户端证书（由后端 `client_ca` 配置项启用）
- `Authorization: Bearer <token>` 头校验，`token` 与 `deviceId` 绑定
- `register` 端点用 body 中的 token 校验（首帧尚未建立 session）

### 5.2 前端 → 后端 HTTP

- 浏览器走 HTTPS（HTTP/2 over TLS，同端口 8080），自签证书需用户在浏览器侧信任
- 可选 Bearer token（如配置启用）
- 由后端配置决定是否强制鉴权

---

## 附录：版本与维护

- 协议版本：`1`（`version` 字段初始值）
- 任何字段变更需同步更新：`firmware/.ino`、`backend/`、`frontend/`、本文档
- 保留 magic `0xF1 0xD0` 用于未来版本兼容性识别
