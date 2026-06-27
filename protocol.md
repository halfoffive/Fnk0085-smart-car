# Fnk0085 智能小车通信协议 v3

> 开发参考协议文档（非用户文档）。锁定设备 ↔ 后端 ↔ 前端的通信契约。
> 任何字段变更必须同步更新本文档与三方实现（firmware/.ino、backend/、frontend/）。

---

## 1. 视频帧上传（设备 → 后端）

视频流走 HTTP/HTTPS（HTTP/1.1 keep-alive），与控制平面共用同一端口（默认 8080）。设备端按 `server` 配置字段 scheme 选择 `WiFiClient`（明文，直连后端）或 `WiFiClientSecure::setInsecure()`（信任所有证书，经 nginx 反代）。后端明文 HTTP（启 `http2` feature 支持 h2c 协商，HTTP/1.1 回退）；对外由 nginx 反代终止 TLS（HTTP/2 over TLS + HTTP/3 over QUIC）。控制指令与事件走第 2 节端点。

### 1.1 端点

| 方法 | 路径 | 用途 |
|------|------|------|
| POST | `/api/device/{deviceId}/frame` | 单帧 JPEG 上传，body 为原始 JPEG 二进制 |

### 1.2 请求头

| 头 | 必填 | 说明 |
|------|------|------|
| `Authorization` | 是 | `Bearer <token>`，与 `deviceId` 绑定 |
| `Content-Type` | 是 | 固定 `image/jpeg` |
| `X-Device-Uptime-Ms` | 是 | 设备系统运行时间（毫秒），用于前端延时计算 |

### 1.3 请求 Body

原始 JPEG 二进制（QVGA 320×240，`jpeg_quality=5`，单帧约 15-30 KB）。**无分包、无 magic、无 framing 头**，HTTP/1.1 keep-alive 复用客户端连接（`WiFiClient` 或 `WiFiClientSecure`）。

### 1.4 响应

| 状态码 | 含义 |
|------|------|
| 204 No Content | 成功（无 body） |
| 401 Unauthorized | token 错误 |
| 404 Not Found | 设备未知（未注册） |
| 413 Payload Too Large | body 超出后端单帧上限 |

### 1.5 失败策略

POST 失败（连接异常或非 2xx）时，固件**直接丢弃该帧不重试**，`videoTask` 立即进入下一帧采集，保证 10fps 节奏不塌。失败计数每 10 帧汇总一次写入日志。`pollTask` 控制通道不受影响。

---

## 2. 设备控制通道（设备 ↔ 后端）

控制平面走 HTTP/HTTPS（HTTP/1.1 或 HTTP/2 h2c，同端口 8080）。设备端按 `server` 配置字段 scheme 选择 `WiFiClient`（明文，直连后端）或 `WiFiClientSecure::setInsecure()`（信任所有证书，经 nginx 反代）。

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

视频流走 `multipart/x-mixed-replace; boundary="fnk0085frame"`（HTTP/1.1 chunked），与后端 `handlers/stream.rs` 实现一致。每个 part 为一帧完整 JPEG：

```
--fnk0085frame\r\n
Content-Type: image/jpeg\r\n
X-Latency-Ms: <int>\r\n
\r\n
<JPEG bytes>\r\n
```

- `X-Latency-Ms`：本帧端到端延迟（ms），由后端计算 `now - deviceUptimeMs` 后透传至前端
- 浏览器通过 `fetch` + `ReadableStream` 或 `<img>` 自动刷新消费

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

### 5.1 设备 → 后端 HTTP/HTTPS

- 后端走明文 HTTP（默认 8080，启 `http2` feature 支持 h2c 协商，HTTP/1.1 回退）；TLS 由 nginx 反代统一处理（HTTP/2 over TLS + HTTP/3 over QUIC），后端不感知 TLS
- 设备端按 `server` 配置字段 scheme 选择客户端：
  - `http://` → `WiFiClient`（明文，直连后端内网端口，延时最低）
  - `https://` → `WiFiClientSecure` + `setInsecure()`（信任所有证书，开发期方案；生产由 nginx 提供合法证书，固件无需感知 CA 轮换）
  - 兼容无 scheme 老配置：默认走 HTTPS
- 固件 `WiFi.config(INADDR_NONE, INADDR_NONE, INADDR_NONE, IPAddress(119,29,29,29), IPAddress(8,8,8,8))` 在 `WiFi.begin()` 前调用（5 参数版：DHCP 自动获取 IP/Gateway/Subnet，仅显式指定 DNS1+DNS2）。`119.29.29.29` 是 DNSPod 国内 DNS，`8.8.8.8` 是 Google 备份
- `Authorization: Bearer <token>` 头校验，`token` 与 `deviceId` 绑定
- `register` 端点用 body 中的 token 校验（首帧尚未建立 session）
- SNTP 时间同步（`configTime(0, "pool.ntp.org", "time.google.com")`，5s 超时失败仅告警）作为日志辅助；`setInsecure()` 跳过证书时间校验，SNTP 失败不影响 HTTPS 握手

### 5.2 前端 → 后端 HTTP

- 浏览器走 HTTPS（nginx 反代，HTTP/2 over TLS + HTTP/3 over QUIC），nginx 提供合法证书，无需浏览器侧手动信任
- 可选 Bearer token（如配置启用）
- 由后端配置决定是否强制鉴权

---

## 附录：版本与维护

- 协议版本：`3`（移除后端 TLS + 固件 HTTP/HTTPS 双模式 + DNS 119.29.29.29 后递增）
- 任何字段变更需同步更新：`firmware/.ino`、`backend/`、`frontend/`、本文档
- 视频帧走 POST（`/api/device/{id}/frame`），body 原始 JPEG，header `Authorization` / `Content-Type` / `X-Device-Uptime-Ms`
- HTTP/HTTPS 设备指令走 `DeviceCommand` enum（tag=`type`，字段 camelCase），事件走 `DeviceEvent` enum（同命名约定），新增 variant 时三端同步
