/* =================================================================
 * Fnk0085-smart-car.ino —— ESP32-S3 智能小车固件主入口
 * -----------------------------------------------------------------
 * 硬件平台：Freenove ESP32-S3 WROOM（带 PSRAM） + OV2640 + L298N + 槽型对射编码器
 * 功能模块：
 *   2.1  摄像头初始化（QVGA/JPEG/PSRAM，10fps）
 *   2.2  L298N 双电机驱动（LEDC PWM，纯函数 setMotor）
 *   2.3  双编码器测速（中断计数 + 100ms 采样 → RPM）
 *   2.4  PID 软件平衡（纯函数 computePid 收敛后写缓存表）
 *   2.5  PWM 缓存表（命中跳过 PID）
 *   2.6  设备身份生成（chipId + MAC）
 *   2.7  UDP+AEAD 视频分包发送（UDP + AES-128-GCM，安全属性等同 DTLS）
 *   2.8  HTTPS 控制通道：长轮询拉取指令 + POST 上报事件（WiFiClientSecure）
 *   2.9  拍照（UXGA + quality=4 写 SD）
 *   2.10 Web Serial 配网（CONFIG|... 行协议 + NVS，串口打印 token）
 *   2.11 WiFi STA + HTTPS/UDP 双通道（HTTPS 控制平面 + UDP 视频平面）
 *   2.12 函数式风格 + 中文注释
 * -----------------------------------------------------------------
 * 控制平面架构（HTTPS）：
 *   - POST /api/device/{id}/register           注册（token 校验）
 *   - GET  /api/device/{id}/poll?timeout=30    长轮询拉指令（返回 cmd JSON 或 Ping）
 *   - POST /api/device/{id}/event              上报 photo_done/ack/error
 *   - 信任自签证书：WiFiClientSecure::setInsecure()（生产部署应换 mTLS / CA pinning）
 *   - pollTask（FreeRTOS，core 0）独占长轮询；指令通过 FreeRTOS 队列投递给 loop
 *   - loop（core 1）消费队列 + 100ms 编码器/PID 周期 + WiFi 重连
 *
 * 视频平面架构（UDP+AEAD）：
 *   Arduino-ESP32 没有官方高层 DTLS API，本固件采用 UDP + 应用层 AES-128-GCM
 *   AEAD（AAD=包头，明文=JPEG 切片）等价实现 DTLS 的机密性 + 完整性 + 认证。
 *   密钥由 token 经 SHA-256 截前 16B 派生，nonce 每包随机 12B，tag 16B 附在
 *   密文后。包头明文以便后端识别 deviceId 与分包序号，与 DTLS record 结构等价。
 * ================================================================= */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiClientSecure.h>
#include <WiFiUdp.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <ArduinoJson.h>
#include <SD_MMC.h>
#include <FS.h>
#include <esp_camera.h>
#include <esp_system.h>
#include <esp_efuse.h>
#include <mbedtls/gcm.h>
#include <mbedtls/sha256.h>
// 目标板：Freenove ESP32-S3 WROOM（含摄像头），对应 esp-camera 的 ESP32S3_EYE 引脚映射
#define CAMERA_MODEL_ESP32S3_EYE
#include "camera_pins.h"

/* =================================================================
 * 全局配置常量（const，编译期确定）
 * ================================================================= */

// L298N 引脚（与 task 描述一致；与摄像头 SIOD/SIOC/VSYNC/HREF 存在硬件冲突，
// 见 camera_pins.h 顶部说明）
const int PIN_IN1 = 4;
const int PIN_IN2 = 5;
const int PIN_IN3 = 6;
const int PIN_IN4 = 7;
const int PIN_ENA = 1;   // 左电机 PWM
const int PIN_ENB = 2;   // 右电机 PWM

// LEDC PWM 配置（core 3.x 由 ledcAttach 自动分配 channel，无需手动指定）
const uint32_t LEDC_FREQ_HZ   = 1000;   // 1 kHz
const uint8_t  LEDC_RES_BITS  = 8;      // 8 位分辨率
const uint16_t PWM_MAX         = 255;   // 8 位 PWM 上限

// 编码器引脚（LEFT_ENC=GPIO14，RIGHT_ENC=GPIO15；GPIO15 与摄像头 XCLK 冲突）
const int      PIN_ENC_LEFT   = 14;
const int      PIN_ENC_RIGHT  = 15;
const uint32_t PULSES_PER_REV = 20;     // 编码器每圈脉冲数（按实际轮调整）
const uint32_t ENC_SAMPLE_MS  = 100;    // 100ms 采样一次 → RPM 计算窗口

// PID 参数（初值，可按实际电机调整）
const float    PID_KP               = 2.0f;
const float    PID_KI               = 0.5f;
const float    PID_KD               = 0.1f;
const int      PID_CONVERGE_N       = 5;     // 连续 5 次稳定则写入缓存
const uint32_t PID_RPM_THRESHOLD    = 5;     // |左-右| < 5 RPM 视为收敛
const int      MAX_TARGET_RPM       = 100;   // PWM=255 对应目标 RPM（经验值）

// 视频
const uint32_t VIDEO_FRAME_INTERVAL_MS = 100; // 100ms/帧 → 10fps
const uint16_t UDP_MTU                 = 1400; // 单 UDP 包最大字节

// 协议常量（与 protocol.md 一致）
const uint8_t  PROTO_MAGIC0     = 0xF1;
const uint8_t  PROTO_MAGIC1     = 0xD0;
const uint8_t  PROTO_VERSION    = 1;
const uint8_t  PROTO_PART_TOTAL = 8;

// 后端默认端口（NVS 未指定 server 时用作回退；正常流程由 CONFIG|server=host:port 注入）
const uint16_t BACKEND_HTTPS_PORT = 8080;  // HTTPS 控制通道端口
const uint16_t BACKEND_UDP_PORT   = 7000;  // UDP 视频流端口（与后端 udp_addr 默认一致）

// HTTPS 长轮询参数
const uint16_t POLL_TIMEOUT_S    = 30;    // 长轮询秒数（后端上限 60）
const uint16_t POLL_TASK_STACK   = 8192;  // pollTask 栈字节
const uint16_t POLL_HTTP_TIMEOUT_MS = 35000; // HTTP 整体超时（略大于 poll 超时）
const uint16_t POLL_BACKOFF_MS   = 1000; // poll 失败后重试间隔

// 指令队列（pollTask → loop）容量
const uint8_t  CMD_QUEUE_LEN     = 8;

// SD_MMC 引脚（ESP32-S3 WROOM 板载硬连线，与 Freenove 示例一致，请勿修改）
const uint8_t  SD_MMC_CLK_PIN   = 39;
const uint8_t  SD_MMC_CMD_PIN   = 38;
const uint8_t  SD_MMC_D0_PIN    = 40;

// NVS 命名空间与键
const char* NVS_NAMESPACE = "fnkcfg";
const char* PHOTO_DIR     = "/photo";

/* =================================================================
 * 数据结构
 * ================================================================= */

// 电机左右 PWM（computePid 的输出）
struct MotorPWM {
  int left;
  int right;
};

// PID 状态（传入传出，函数式风格不修改全局）
struct PidState {
  float leftIntegral;
  float rightIntegral;
  float prevLeftError;
  float prevRightError;
  int   convergeCount;
};

// PWM 缓存条目
struct PwmCacheEntry {
  int  targetSpeed;   // 目标 RPM
  int  stablePwm;     // 收敛后的稳定 PWM（左右一致）
  bool valid;
};

/* =================================================================
 * 全局可变状态（仅限：volatile 计数器、PWM 缓存表、pwmCacheEnabled 开关、
 *              帧序号、运动状态、网络句柄、设备身份）
 * ================================================================= */

// 编码器中断计数（volatile，ISR 访问）
volatile uint32_t encLeftCount  = 0;
volatile uint32_t encRightCount = 0;
// 100ms 采样快照（供 getLeftRpm/getRightRpm 读取）
volatile uint32_t encLeftSnapshot  = 0;
volatile uint32_t encRightSnapshot = 0;
portMUX_TYPE encMux = portMUX_INITIALIZER_UNLOCKED;

// PWM 缓存表（targetSpeed → stablePwm）
const int PWM_CACHE_SIZE = 16;
// 显式初始化全部字段，避免 -Wmissing-field-initializers
PwmCacheEntry pwmCache[PWM_CACHE_SIZE] = { {0, 0, false} };
bool pwmCacheEnabled = true;   // 全局开关，可被 pwm_cache 指令切换

// 帧序号（每帧 +1，AES-GCM nonce 一部分）
volatile uint32_t frameSeq = 0;

// 运动状态（由 control 指令设置，loop 周期消费）
String    targetDirection = "stop";
int       targetPwm       = 0;
uint32_t  motionStopAt    = 0;     // 0 表示不自动停止
PidState  pidState        = { 0.0f, 0.0f, 0.0f, 0.0f, 0 };

// 设备身份
String deviceId;
String deviceToken;
String backendHost;        // HTTPS 控制通道 host
uint16_t backendHttpsPort = BACKEND_HTTPS_PORT;  // HTTPS 端口

// 网络：UDP 仅用于视频流；HTTPS 用于控制通道
WiFiUDP             udpSend;        // 视频流发送
WiFiClientSecure    httpsClient;    // 控制通道（信任自签证书）
QueueHandle_t        cmdQueue = NULL;  // pollTask → loop 的指令队列
TaskHandle_t         pollTaskHandle = NULL;

// AES-128-GCM 密钥（token 派生）
uint8_t aesKey[16];

// 拍照互斥（防止视频任务与拍照竞争传感器）
portMUX_TYPE photoMux = portMUX_INITIALIZER_UNLOCKED;
volatile bool photoInProgress = false;

// 视频任务句柄（vTaskSuspend/Resume 用）
TaskHandle_t videoTaskHandle = NULL;

/* =================================================================
 * 前置声明
 * ================================================================= */
void cameraInit();
void motorInit();
void encoderInit();
void encoderSample();
uint32_t getLeftRpm();
uint32_t getRightRpm();
void setMotor(int leftPwm, int rightPwm);
MotorPWM computePid(int targetSpeed, uint32_t leftRpm, uint32_t rightRpm,
                    const PidState& prev, PidState& next);
bool pwmCacheLookup(int targetSpeed, int& outPwm);
void pwmCacheStore(int targetSpeed, int stablePwm);
void setPwmCacheEnabled(bool enabled);
String generateDeviceId();
void deriveAesKey(const String& token, uint8_t outKey[16]);
bool aeadEncrypt(const uint8_t* key, const uint8_t* nonce12,
                 const uint8_t* aad, size_t aadLen,
                 const uint8_t* plain, size_t plainLen,
                 uint8_t* cipher, uint8_t tagOut[16]);
void sendVideoFrame(uint8_t* jpegBuf, size_t jpegLen, uint32_t uptimeMs);

// HTTPS 控制通道
int  httpsPost(const String& path, const String& body, String& respOut);
int  httpsGet(const String& path, String& respOut);
void sendRegister();
void sendPhotoDone(const String& path, uint32_t uptimeMs);
void sendAck(int refSeq);
void sendError(int code, const String& msg);
void pollTask(void* arg);
void dispatchCommands();

// 指令处理器
void handleControl(const JsonDocument& doc);
void handlePhoto();
void handlePwmCache(const JsonDocument& doc);
void handlePing(const JsonDocument& doc);

// Web Serial 配网
void pollSerialConfig();
bool parseConfigLine(const String& line, String& ssid, String& password,
                     String& server, String& token);
bool loadConfigFromNVS(String& ssid, String& password, String& server, String& token);
void saveConfigToNVS(const String& ssid, const String& password,
                     const String& server, const String& token);
void printStoredConfig();
void videoTask(void* arg);

/* =================================================================
 * 2.1 摄像头初始化（参考 Sketch_07.2）
 * ================================================================= */
void cameraInit() {
  camera_config_t config;
  config.ledc_channel = LEDC_CHANNEL_0;   // 摄像头占用 channel 0
  config.ledc_timer   = LEDC_TIMER_0;
  config.pin_d0  = Y2_GPIO_NUM;
  config.pin_d1  = Y3_GPIO_NUM;
  config.pin_d2  = Y4_GPIO_NUM;
  config.pin_d3  = Y5_GPIO_NUM;
  config.pin_d4  = Y6_GPIO_NUM;
  config.pin_d5  = Y7_GPIO_NUM;
  config.pin_d6  = Y8_GPIO_NUM;
  config.pin_d7  = Y9_GPIO_NUM;
  config.pin_xclk  = XCLK_GPIO_NUM;
  config.pin_pclk  = PCLK_GPIO_NUM;
  config.pin_vsync = VSYNC_GPIO_NUM;
  config.pin_href  = HREF_GPIO_NUM;
  config.pin_sccb_sda = SIOD_GPIO_NUM;
  config.pin_sccb_scl = SIOC_GPIO_NUM;
  config.pin_pwdn  = PWDN_GPIO_NUM;
  config.pin_reset = RESET_GPIO_NUM;

  config.xclk_freq_hz = 10000000;                 // 10 MHz
  config.frame_size   = FRAMESIZE_QVGA;           // 320x240
  config.pixel_format = PIXFORMAT_JPEG;
  config.grab_mode    = CAMERA_GRAB_WHEN_EMPTY;
  config.fb_location  = CAMERA_FB_IN_PSRAM;
  config.jpeg_quality = 10;
  config.fb_count     = 1;

  esp_err_t err = esp_camera_init(&config);
  if (err != ESP_OK) {
    Serial.printf("[CAM] init failed: 0x%x\n", err);
    return;
  }

  // OV2640 设置 hmirror/vflip（参考 Sketch_07.2）
  sensor_t* s = esp_camera_sensor_get();
  if (s && s->id.PID == OV2640_PID) {
    s->set_hmirror(s, 1);
    s->set_vflip(s, 1);
  }
  Serial.println("[CAM] init ok (QVGA/JPEG/PSRAM)");
}

/* =================================================================
 * 2.2 L298N 双电机驱动
 * ================================================================= */
void motorInit() {
  pinMode(PIN_IN1, OUTPUT);
  pinMode(PIN_IN2, OUTPUT);
  pinMode(PIN_IN3, OUTPUT);
  pinMode(PIN_IN4, OUTPUT);
  // Arduino-ESP32 core 3.x：ledcAttach 一步完成 channel 分配 + 引脚绑定 + 频率/分辨率设置
  ledcAttach(PIN_ENA, LEDC_FREQ_HZ, LEDC_RES_BITS);
  ledcAttach(PIN_ENB, LEDC_FREQ_HZ, LEDC_RES_BITS);
  setMotor(0, 0);
}

// 纯函数：根据左右 PWM 设置方向引脚并写占空比
// 正 PWM：正向；负 PWM：反向；绝对值为占空比
void setMotor(int leftPwm, int rightPwm) {
  // 左电机方向
  if (leftPwm >= 0) {
    digitalWrite(PIN_IN1, HIGH);
    digitalWrite(PIN_IN2, LOW);
  } else {
    digitalWrite(PIN_IN1, LOW);
    digitalWrite(PIN_IN2, HIGH);
    leftPwm = -leftPwm;
  }
  // 右电机方向
  if (rightPwm >= 0) {
    digitalWrite(PIN_IN3, HIGH);
    digitalWrite(PIN_IN4, LOW);
  } else {
    digitalWrite(PIN_IN3, LOW);
    digitalWrite(PIN_IN4, HIGH);
    rightPwm = -rightPwm;
  }
  // 限幅
  if (leftPwm  > PWM_MAX) leftPwm  = PWM_MAX;
  if (rightPwm > PWM_MAX) rightPwm = PWM_MAX;
  // core 3.x：ledcWrite 第一参数为引脚而非 channel
  ledcWrite(PIN_ENA, (uint32_t)leftPwm);
  ledcWrite(PIN_ENB, (uint32_t)rightPwm);
}

/* =================================================================
 * 2.3 双编码器测速
 * ================================================================= */
void IRAM_ATTR onLeftEncoder() {
  // core 3.x：volatile 的 ++ 已弃用，改用显式赋值
  encLeftCount = encLeftCount + 1;
}
void IRAM_ATTR onRightEncoder() {
  encRightCount = encRightCount + 1;
}

void encoderInit() {
  pinMode(PIN_ENC_LEFT,  INPUT_PULLUP);
  pinMode(PIN_ENC_RIGHT, INPUT_PULLUP);
  attachInterrupt(digitalPinToInterrupt(PIN_ENC_LEFT),  onLeftEncoder,  FALLING);
  attachInterrupt(digitalPinToInterrupt(PIN_ENC_RIGHT), onRightEncoder, FALLING);
}

// 每 100ms 调用：原子地拷贝计数并清零（供 getLeftRpm/getRightRpm 读取）
void encoderSample() {
  portENTER_CRITICAL(&encMux);
  encLeftSnapshot  = encLeftCount;
  encRightSnapshot = encRightCount;
  encLeftCount  = 0;
  encRightCount = 0;
  portEXIT_CRITICAL(&encMux);
}

// 纯函数：读取并清零快照（实际清零在 encoderSample 中完成）
// RPM = (count / pulses_per_rev) * (1000 / sample_ms) * 60
uint32_t getLeftRpm() {
  uint32_t count;
  portENTER_CRITICAL(&encMux);
  count = encLeftSnapshot;
  portEXIT_CRITICAL(&encMux);
  return (count * 1000UL * 60UL) / (PULSES_PER_REV * ENC_SAMPLE_MS);
}

uint32_t getRightRpm() {
  uint32_t count;
  portENTER_CRITICAL(&encMux);
  count = encRightSnapshot;
  portEXIT_CRITICAL(&encMux);
  return (count * 1000UL * 60UL) / (PULSES_PER_REV * ENC_SAMPLE_MS);
}

/* =================================================================
 * 2.4 + 2.5 PID 软件平衡 + PWM 缓存表
 * ================================================================= */

// 纯函数：根据目标速度 + 当前左右 RPM + 上次状态，计算左右 PWM 并输出新状态
// 不修改全局变量；调用方负责把 next 写回 pidState
MotorPWM computePid(int targetSpeed, uint32_t leftRpm, uint32_t rightRpm,
                    const PidState& prev, PidState& next) {
  MotorPWM out;
  int leftError  = targetSpeed - (int)leftRpm;
  int rightError = targetSpeed - (int)rightRpm;

  float leftP  = PID_KP * leftError;
  float rightP = PID_KP * rightError;

  float leftI  = prev.leftIntegral  + PID_KI * leftError;
  float rightI = prev.rightIntegral + PID_KI * rightError;

  float leftD  = PID_KD * (leftError  - prev.prevLeftError);
  float rightD = PID_KD * (rightError - prev.prevRightError);

  int leftPwm  = (int)(leftP  + leftI  + leftD);
  int rightPwm = (int)(rightP + rightI + rightD);

  // 限幅
  if (leftPwm  >  PWM_MAX) leftPwm  =  PWM_MAX;
  if (leftPwm  < -PWM_MAX) leftPwm  = -PWM_MAX;
  if (rightPwm >  PWM_MAX) rightPwm =  PWM_MAX;
  if (rightPwm < -PWM_MAX) rightPwm = -PWM_MAX;

  out.left  = leftPwm;
  out.right = rightPwm;

  next.leftIntegral    = leftI;
  next.rightIntegral   = rightI;
  next.prevLeftError   = leftError;
  next.prevRightError  = rightError;

  // 收敛判定：|leftRpm - rightRpm| < threshold 计数 +1
  int diff = (int)leftRpm - (int)rightRpm;
  if (diff < 0) diff = -diff;
  next.convergeCount = (diff < (int)PID_RPM_THRESHOLD) ? (prev.convergeCount + 1) : 0;

  return out;
}

// 查缓存表
bool pwmCacheLookup(int targetSpeed, int& outPwm) {
  if (!pwmCacheEnabled) return false;
  for (int i = 0; i < PWM_CACHE_SIZE; i++) {
    if (pwmCache[i].valid && pwmCache[i].targetSpeed == targetSpeed) {
      outPwm = pwmCache[i].stablePwm;
      return true;
    }
  }
  return false;
}

// 写缓存表
void pwmCacheStore(int targetSpeed, int stablePwm) {
  // 已存在则更新
  for (int i = 0; i < PWM_CACHE_SIZE; i++) {
    if (pwmCache[i].valid && pwmCache[i].targetSpeed == targetSpeed) {
      pwmCache[i].stablePwm = stablePwm;
      return;
    }
  }
  // 否则占用空槽
  for (int i = 0; i < PWM_CACHE_SIZE; i++) {
    if (!pwmCache[i].valid) {
      pwmCache[i].targetSpeed = targetSpeed;
      pwmCache[i].stablePwm  = stablePwm;
      pwmCache[i].valid      = true;
      return;
    }
  }
  // 表满：覆盖第 0 个（FIFO 简化处理）
  pwmCache[0].targetSpeed = targetSpeed;
  pwmCache[0].stablePwm  = stablePwm;
  pwmCache[0].valid      = true;
}

// 全局开关
void setPwmCacheEnabled(bool enabled) {
  pwmCacheEnabled = enabled;
  Serial.printf("[PID] pwm_cache enabled=%d\n", (int)enabled);
}

/* =================================================================
 * 2.6 设备身份生成
 * ================================================================= */
String generateDeviceId() {
  // ESP32-S3 core 3.x 无 ESP.getChipId；用 EFUSE MAC 低 32 位作为芯片编号
  uint64_t mac64 = ESP.getEfuseMac();
  uint32_t chipId = (uint32_t)(mac64 >> 24) ^ (uint32_t)mac64;
  String mac = WiFi.macAddress();
  mac.replace(":", "");
  String id = "ESP32S3_" + String(chipId, HEX) + "_" + mac;
  id.toUpperCase();
  return id;
}

/* =================================================================
 * 2.7 AEAD 加密（AES-128-GCM，等同 DTLS 安全属性）
 * ================================================================= */

// 由 token 派生 128 位 AES 密钥（SHA-256 截前 16B）
// 注：mbedtls 3.x 移除了 _ret 后缀，新版函数直接返回 int 错误码
void deriveAesKey(const String& token, uint8_t outKey[16]) {
  uint8_t sha[32];
  mbedtls_sha256_context ctx;
  mbedtls_sha256_init(&ctx);
  mbedtls_sha256_starts(&ctx, 0);
  mbedtls_sha256_update(&ctx, (const uint8_t*)token.c_str(), token.length());
  mbedtls_sha256_finish(&ctx, sha);
  mbedtls_sha256_free(&ctx);
  memcpy(outKey, sha, 16);
}

// AES-128-GCM 加密：plain → cipher + 16B tag
// AAD 提供包头完整性绑定（接收端用包头作为 AAD 解密验证）
bool aeadEncrypt(const uint8_t* key, const uint8_t* nonce12,
                 const uint8_t* aad, size_t aadLen,
                 const uint8_t* plain, size_t plainLen,
                 uint8_t* cipher, uint8_t tagOut[16]) {
  mbedtls_gcm_context ctx;
  mbedtls_gcm_init(&ctx);
  int ret = mbedtls_gcm_setkey(&ctx, MBEDTLS_CIPHER_ID_AES, key, 128);
  if (ret != 0) {
    mbedtls_gcm_free(&ctx);
    return false;
  }
  ret = mbedtls_gcm_crypt_and_tag(&ctx, MBEDTLS_GCM_ENCRYPT, plainLen,
                                  nonce12, 12,
                                  aad, aadLen,
                                  plain, cipher,
                                  16, tagOut);
  mbedtls_gcm_free(&ctx);
  return ret == 0;
}

/* =================================================================
 * 2.7 UDP 视频分包发送（按 protocol.md 字节布局打包 8 子包）
 * ================================================================= */

// 将一帧 JPEG 按 byte 均分为 8 段，每段独立 AEAD 加密后 UDP 发送
// row/col 仅作为 2x4 网格位置语义标记，不参与切分
void sendVideoFrame(uint8_t* jpegBuf, size_t jpegLen, uint32_t uptimeMs) {
  if (jpegLen == 0) return;

  uint32_t seq = frameSeq;
  frameSeq = frameSeq + 1;
  const size_t deviceIdLen = deviceId.length();
  if (deviceIdLen > 200) {
    // deviceIdLen 字段是 uint8，超长保护
    return;
  }

  // 按字节均分：前 (PART_TOTAL-1) 段等长，最后一段吸收余数
  size_t baseLen  = jpegLen / PROTO_PART_TOTAL;
  size_t remainder = jpegLen % PROTO_PART_TOTAL;
  size_t offset = 0;

  // 临时缓冲：包头 + nonce + 密文 + tag
  static uint8_t packetBuf[1600];
  static uint8_t cipherBuf[UDP_MTU];
  uint8_t nonce[12];
  uint8_t tag[16];

  for (uint8_t p = 0; p < PROTO_PART_TOTAL; p++) {
    size_t partLen = baseLen + (p < remainder ? 1 : 0);
    if (p == PROTO_PART_TOTAL - 1) {
      partLen = jpegLen - offset;   // 最后一段吸收余数（防御性）
    }
    if (partLen == 0) continue;

    uint8_t row = p / 4;   // 0..1
    uint8_t col = p % 4;   // 0..3

    // ---- 构造包头（明文，作为 AAD）----
    size_t off = 0;
    packetBuf[off++] = PROTO_MAGIC0;
    packetBuf[off++] = PROTO_MAGIC1;
    packetBuf[off++] = PROTO_VERSION;
    packetBuf[off++] = (uint8_t)deviceIdLen;
    memcpy(packetBuf + off, deviceId.c_str(), deviceIdLen);
    off += deviceIdLen;
    // uptimeMs 8B LE
    for (int i = 0; i < 8; i++) {
      packetBuf[off++] = (uint8_t)((uptimeMs >> (i * 8)) & 0xFF);
    }
    // frameSeq 4B LE
    for (int i = 0; i < 4; i++) {
      packetBuf[off++] = (uint8_t)((seq >> (i * 8)) & 0xFF);
    }
    packetBuf[off++] = p;                  // partIdx
    packetBuf[off++] = PROTO_PART_TOTAL;   // partTotal
    packetBuf[off++] = row;                 // row
    packetBuf[off++] = col;                 // col
    // payloadLen 4B LE（明文长度 M，接收端据此读 M+28 字节）
    uint32_t payloadLen = (uint32_t)partLen;
    for (int i = 0; i < 4; i++) {
      packetBuf[off++] = (uint8_t)((payloadLen >> (i * 8)) & 0xFF);
    }
    size_t hdrLen = off;

    // ---- 生成 12B 随机 nonce ----
    esp_fill_random(nonce, 12);

    // ---- AEAD 加密 ----
    if (partLen > UDP_MTU - 16) {
      // 单段过大，跳过（QVGA quality=10 不会触发）
      offset += partLen;
      continue;
    }
    if (!aeadEncrypt(aesKey, nonce,
                    packetBuf, hdrLen,            // AAD = 包头
                    jpegBuf + offset, partLen,    // 明文 = JPEG 切片
                    cipherBuf, tag)) {
      Serial.println("[NET] AEAD encrypt failed, skip part");
      offset += partLen;
      continue;
    }

    // ---- 发送：包头 + nonce(12) + cipher(M) + tag(16) ----
    udpSend.beginPacket(backendHost.c_str(), BACKEND_UDP_PORT);
    udpSend.write(packetBuf, hdrLen);
    udpSend.write(nonce, 12);
    udpSend.write(cipherBuf, partLen);
    udpSend.write(tag, 16);
    udpSend.endPacket();

    offset += partLen;
  }
}

/* =================================================================
 * 2.8 HTTPS 控制通道：register / event / poll
 * -----------------------------------------------------------------
 * - WiFiClientSecure + setInsecure() 信任后端自签证书（开发期方案）。
 *   生产部署应改为 mTLS 或 CA pinning（ setCACert() ）。
 * - httpsPost/httpsGet 复用全局 httpsClient；返回 HTTP 状态码，<0 表示网络错误。
 * - pollTask 独占长轮询；loop 仅消费队列。
 * ================================================================= */

// 构造完整 HTTPS URL
String buildUrl(const String& path) {
  String url = "https://";
  url += backendHost;
  url += ":";
  url += String(backendHttpsPort);
  url += path;
  return url;
}

// POST JSON 到指定 path；返回 HTTP 状态码（>=200），<0 为网络/协议错误
int httpsPost(const String& path, const String& body, String& respOut) {
  HTTPClient http;
  String url = buildUrl(path);
  if (!http.begin(httpsClient, url)) {
    Serial.printf("[HTTPS] POST begin failed: %s\n", url.c_str());
    return -1;
  }
  http.addHeader("Content-Type", "application/json");
  http.addHeader("Authorization", "Bearer " + deviceToken);
  http.setTimeout(POLL_HTTP_TIMEOUT_MS);
  int code = http.POST(body);
  if (code > 0) respOut = http.getString();
  http.end();
  return code;
}

// GET 指定 path（长轮询）；返回 HTTP 状态码，<0 为网络/协议错误
int httpsGet(const String& path, String& respOut) {
  HTTPClient http;
  String url = buildUrl(path);
  if (!http.begin(httpsClient, url)) {
    Serial.printf("[HTTPS] GET begin failed: %s\n", url.c_str());
    return -1;
  }
  http.addHeader("Authorization", "Bearer " + deviceToken);
  http.addHeader("Cache-Control", "no-store");
  http.setTimeout(POLL_HTTP_TIMEOUT_MS);
  int code = http.GET();
  if (code > 0) respOut = http.getString();
  http.end();
  return code;
}

// POST /api/device/{id}/register —— body: {"token":"Bearer xxx"}
void sendRegister() {
  JsonDocument doc;
  doc["token"] = "Bearer " + deviceToken;
  String body;
  serializeJson(doc, body);
  String path = "/api/device/" + deviceId + "/register";
  String resp;
  int code = httpsPost(path, body, resp);
  if (code == 200) {
    Serial.printf("[NET] register ok, deviceId=%s\n", deviceId.c_str());
  } else {
    Serial.printf("[NET] register failed code=%d resp=%s\n", code, resp.c_str());
  }
}

// POST /api/device/{id}/event —— body: {"type":"photo_done","path":...,"uptimeMs":...}
void sendPhotoDone(const String& path, uint32_t uptimeMs) {
  JsonDocument doc;
  doc["type"]     = "photo_done";
  doc["path"]     = path;
  doc["uptimeMs"] = uptimeMs;
  String body;
  serializeJson(doc, body);
  String url = "/api/device/" + deviceId + "/event";
  String resp;
  int code = httpsPost(url, body, resp);
  if (code != 200) {
    Serial.printf("[NET] photo_done failed code=%d\n", code);
  }
}

// POST /api/device/{id}/event —— body: {"type":"ack","refSeq":N}
void sendAck(int refSeq) {
  JsonDocument doc;
  doc["type"]   = "ack";
  doc["refSeq"] = refSeq;
  String body;
  serializeJson(doc, body);
  String url = "/api/device/" + deviceId + "/event";
  String resp;
  int code = httpsPost(url, body, resp);
  if (code != 200) {
    Serial.printf("[NET] ack failed code=%d\n", code);
  }
}

// POST /api/device/{id}/event —— body: {"type":"error","code":N,"message":...}
void sendError(int code, const String& msg) {
  JsonDocument doc;
  doc["type"]    = "error";
  doc["code"]    = code;
  doc["message"] = msg;
  String body;
  serializeJson(doc, body);
  String url = "/api/device/" + deviceId + "/event";
  String resp;
  int httpCode = httpsPost(url, body, resp);
  if (httpCode != 200) {
    Serial.printf("[NET] error report failed code=%d\n", httpCode);
  }
}

// pollTask：长轮询 GET /api/device/{id}/poll?timeout=N
// 收到指令 JSON 推入 cmdQueue；收到 Ping（超时占位）忽略；失败按 POLL_BACKOFF_MS 退避
void pollTask(void* arg) {
  String path = "/api/device/" + deviceId + "/poll?timeout=" + String(POLL_TIMEOUT_S);
  for (;;) {
    if (WiFi.status() != WL_CONNECTED) {
      vTaskDelay(pdMS_TO_TICKS(POLL_BACKOFF_MS));
      continue;
    }
    String resp;
    int code = httpsGet(path, resp);
    if (code != 200) {
      // 网络错误或 5xx：退避后重试
      Serial.printf("[POLL] code=%d, backoff %u ms\n", code, POLL_BACKOFF_MS);
      vTaskDelay(pdMS_TO_TICKS(POLL_BACKOFF_MS));
      continue;
    }
    if (resp.length() == 0) {
      // 空响应（不应发生）：直接进入下一轮
      continue;
    }
    // 解析 type 字段决定是否入队
    JsonDocument doc;
    DeserializationError err = deserializeJson(doc, resp);
    if (err != DeserializationError::Ok) {
      Serial.printf("[POLL] JSON parse failed: %s\n", err.c_str());
      vTaskDelay(pdMS_TO_TICKS(POLL_BACKOFF_MS));
      continue;
    }
    String type = doc["type"] | "";
    if (type == "ping") {
      // 后端长轮询超时占位；立即发起下一次 poll
      continue;
    }
    // 真实指令：复制到堆上投递给 loop
    String* p = new String(resp);
    if (p == nullptr) {
      Serial.println("[POLL] alloc failed, drop");
      continue;
    }
    if (xQueueSend(cmdQueue, &p, 0) != pdPASS) {
      Serial.println("[POLL] queue full, drop cmd");
      delete p;
    }
  }
}

// 主 loop 调用：从 cmdQueue 拉取所有可用指令并派发到 handler
void dispatchCommands() {
  if (cmdQueue == NULL) return;
  String* p = nullptr;
  while (xQueueReceive(cmdQueue, &p, 0) == pdPASS) {
    if (p == nullptr) continue;
    JsonDocument doc;
    DeserializationError err = deserializeJson(doc, *p);
    if (err == DeserializationError::Ok) {
      String type = doc["type"] | "";
      if      (type == "control")   handleControl(doc);
      else if (type == "photo")     handlePhoto();
      else if (type == "pwm_cache") handlePwmCache(doc);
      else if (type == "ping")      handlePing(doc);
      else Serial.printf("[NET] unknown cmd type: %s\n", type.c_str());
    } else {
      Serial.printf("[NET] cmd JSON parse failed: %s\n", err.c_str());
    }
    delete p;
  }
}

/* =================================================================
 * 视频采集任务（FreeRTOS，core 0）
 * 通过 vTaskDelayUntil 节奏控制 10fps
 * ================================================================= */
void videoTask(void* arg) {
  TickType_t lastWake = xTaskGetTickCount();
  for (;;) {
    vTaskDelayUntil(&lastWake, pdMS_TO_TICKS(VIDEO_FRAME_INTERVAL_MS));
    // 拍照进行中跳过（双重保护：vTaskSuspend + 此标志）
    if (photoInProgress) continue;

    camera_fb_t* fb = esp_camera_fb_get();
    if (!fb) continue;
    sendVideoFrame(fb->buf, fb->len, millis());
    esp_camera_fb_return(fb);
  }
}

/* =================================================================
 * 指令处理器（由 dispatchCommands 调用，doc 已解析）
 * ================================================================= */

// type=control：设置运动状态（loop 周期消费 → PID）
void handleControl(const JsonDocument& doc) {
  String dir = doc["direction"] | "stop";
  int pwm = doc["pwm"] | 0;
  uint32_t durationMs = doc["durationMs"] | (uint32_t)0;

  if (pwm < 0) pwm = 0;
  if (pwm > PWM_MAX) pwm = PWM_MAX;

  targetDirection = dir;
  targetPwm = pwm;

  // 立即响应（停止/转弯不需要 PID）
  if (dir == "S") {
    setMotor(-pwm, -pwm);
  } else if (dir == "A") {
    setMotor(-pwm, pwm);
  } else if (dir == "D") {
    setMotor(pwm, -pwm);
  } else if (dir == "stop") {
    setMotor(0, 0);
    memset(&pidState, 0, sizeof(pidState));
  }
  // "W" 由 loop 中 PID 周期处理

  if (durationMs > 0) {
    motionStopAt = millis() + durationMs;
  } else {
    motionStopAt = 0;
  }
}

// type=photo：UXGA 拍照写 SD
void handlePhoto() {
  // 1. 暂停视频流任务（防止竞争传感器）
  if (videoTaskHandle) vTaskSuspend(videoTaskHandle);
  photoInProgress = true;

  // 2. 切换 UXGA + quality=4
  sensor_t* s = esp_camera_sensor_get();
  if (s) {
    s->set_framesize(s, FRAMESIZE_UXGA);
    s->set_quality(s, 4);
  }

  // 3. 采集一帧
  camera_fb_t* fb = esp_camera_fb_get();
  uint32_t uptime = millis();

  // 4. 写 SD
  bool ok = false;
  String path = "";
  if (fb) {
    path = String(PHOTO_DIR) + "/photo_" + String(uptime) + ".jpg";
    File f = SD_MMC.open(path, FILE_WRITE);
    if (f) {
      f.write(fb->buf, fb->len);
      f.close();
      ok = true;
    } else {
      Serial.println("[PHOTO] open file failed");
    }
    esp_camera_fb_return(fb);
  } else {
    Serial.println("[PHOTO] capture failed");
  }

  // 5. 恢复 QVGA + quality=10
  if (s) {
    s->set_framesize(s, FRAMESIZE_QVGA);
    s->set_quality(s, 10);
  }

  // 6. 恢复视频流任务
  photoInProgress = false;
  if (videoTaskHandle) vTaskResume(videoTaskHandle);

  // 7. 发 photo_done 回执
  if (ok) {
    sendPhotoDone(path, uptime);
  } else {
    sendError(5002, "photo capture or sd write failed");
  }
}

// type=pwm_cache：切换缓存开关
void handlePwmCache(const JsonDocument& doc) {
  bool enabled = doc["enabled"] | true;
  setPwmCacheEnabled(enabled);
}

// type=ping：回 ack
void handlePing(const JsonDocument& doc) {
  int seq = doc["seq"] | 0;
  sendAck(seq);
}

/* =================================================================
 * 2.10 Web Serial 配网命令解析
 * ================================================================= */

// 解析 CONFIG|ssid=<ssid>|password=<pwd>|server=<host:port>|token=<token>\n
bool parseConfigLine(const String& line, String& ssid, String& password,
                     String& server, String& token) {
  if (!line.startsWith("CONFIG|")) return false;
  String rest = line.substring(7);
  int idx = 0;
  while (idx < (int)rest.length()) {
    int sep = rest.indexOf('|', idx);
    String kv = (sep < 0) ? rest.substring(idx) : rest.substring(idx, sep);
    int eq = kv.indexOf('=');
    if (eq > 0) {
      String k = kv.substring(0, eq);
      String v = kv.substring(eq + 1);
      if      (k == "ssid")     ssid = v;
      else if (k == "password") password = v;
      else if (k == "server")   server = v;
      else if (k == "token")    token = v;
    }
    if (sep < 0) break;
    idx = sep + 1;
  }
  return ssid.length() > 0 && server.length() > 0 && token.length() > 0;
}

bool loadConfigFromNVS(String& ssid, String& password, String& server, String& token) {
  Preferences p;
  p.begin(NVS_NAMESPACE, true);
  ssid     = p.getString("ssid", "");
  password = p.getString("password", "");
  server   = p.getString("server", "");
  token    = p.getString("token", "");
  p.end();
  return ssid.length() > 0 && server.length() > 0 && token.length() > 0;
}

void saveConfigToNVS(const String& ssid, const String& password,
                     const String& server, const String& token) {
  Preferences p;
  p.begin(NVS_NAMESPACE, false);
  p.putString("ssid", ssid);
  p.putString("password", password);
  p.putString("server", server);
  p.putString("token", token);
  p.end();
}

// 打印 NVS 中已存的配置（用于调试配网问题；password 仅打长度避免泄露）
void printStoredConfig() {
  String ssid, password, server, token;
  if (!loadConfigFromNVS(ssid, password, server, token)) {
    Serial.println("[CFG] NVS empty (no valid config)");
    return;
  }
  Serial.println("==================================================");
  Serial.println("[CFG] stored NVS config:");
  Serial.printf("       ssid    = %s\n", ssid.c_str());
  Serial.printf("       password len = %d\n", password.length());
  Serial.printf("       server  = %s\n", server.c_str());
  Serial.printf("       token   = %s\n", token.c_str());
  Serial.println("==================================================");
}

// 持续监听 Serial，解析 CONFIG 行；成功 → OK → REBOOT
void pollSerialConfig() {
  static String line;
  while (Serial.available()) {
    char c = Serial.read();
    if (c == '\n') {
      if (line.startsWith("CONFIG|")) {
        Serial.printf("[CFG] recv: %s\n", line.c_str());
        String ssid, password, server, token;
        if (parseConfigLine(line, ssid, password, server, token)) {
          // 校验 server 含 ':' 且端口在 1..65535
          int colon = server.indexOf(':');
          if (colon <= 0) {
            Serial.println("ERR|invalid_server (missing :port)");
          } else {
            long port = server.substring(colon + 1).toInt();
            if (port <= 0 || port > 65535) {
              Serial.println("ERR|invalid_server (port out of range)");
            } else {
              // 写 NVS
              saveConfigToNVS(ssid, password, server, token);
              Serial.println("[CFG] NVS saved, rebooting...");
              Serial.println("OK");
              Serial.println("REBOOT");
              Serial.flush();
              delay(100);
              ESP.restart();
            }
          }
        } else {
          Serial.println("ERR|invalid_format (need ssid|password|server|token)");
        }
      } else if (line == "CONFIG") {
        // 查询命令：打印当前 NVS 配置（含 token）
        printStoredConfig();
      } else if (line.length() > 0) {
        // 非空且非 CONFIG 行：忽略
        Serial.printf("ERR|unknown_command: %s\n", line.c_str());
      }
      line = "";
    } else if (c != '\r') {
      line += c;
      // 防止缓冲无限增长
      if (line.length() > 512) line = "";
    }
  }
}

/* =================================================================
 * setup / loop
 * ================================================================= */
void setup() {
  Serial.begin(115200);
  Serial.setDebugOutput(true);
  delay(300);
  Serial.println();
  Serial.println("==================================================");
  Serial.println("[BOOT] Fnk0085 smart car firmware booting");
  Serial.println("==================================================");

  // 2.1 摄像头
  cameraInit();

  // SD 卡（SDMMC，1-bit 模式 + setPins，与 Freenove 示例一致）
  // ESP32-S3 SD_MMC 必须先 setPins 再 begin，否则走默认引脚导致挂载失败
  SD_MMC.setPins(SD_MMC_CLK_PIN, SD_MMC_CMD_PIN, SD_MMC_D0_PIN);
  if (!SD_MMC.begin("/sdcard", true, true, SDMMC_FREQ_DEFAULT, 5)) {
    Serial.println("[SD] mount failed (photo unavailable)");
  } else {
    uint8_t cardType = SD_MMC.cardType();
    const char* typeName =
      (cardType == CARD_NONE)   ? "NONE"   :
      (cardType == CARD_MMC)    ? "MMC"    :
      (cardType == CARD_SD)     ? "SDSC"   :
      (cardType == CARD_SDHC)   ? "SDHC"   :
                                 "UNKNOWN";
    uint64_t cardSizeMB = SD_MMC.cardSize() / (1024 * 1024);
    Serial.printf("[SD] mount ok, type=%s, size=%lluMB, total=%lluMB\n",
                  typeName, cardSizeMB,
                  SD_MMC.totalBytes() / (1024 * 1024));
    if (!SD_MMC.exists(PHOTO_DIR)) {
      SD_MMC.mkdir(PHOTO_DIR);
    }
  }

  // 2.2 电机
  motorInit();

  // 2.3 编码器
  encoderInit();

  // 2.10 加载 NVS 配置；若无 → 等待 Web Serial 配网
  String ssid, password, server, token;
  if (!loadConfigFromNVS(ssid, password, server, token)) {
    Serial.println("[CFG] NVS empty, waiting for Web Serial provisioning...");
    Serial.println("      Send: CONFIG|ssid=<ssid>|password=<pwd>|server=<host:port>|token=<token>\\n");
    while (true) {
      pollSerialConfig();
      delay(10);
    }
  }

  // 解析 server="host:port"（HTTPS 端口，默认 8080）
  int colon = server.indexOf(':');
  if (colon <= 0) {
    Serial.println("[CFG] invalid server in NVS, waiting for re-provisioning");
    while (true) { pollSerialConfig(); delay(10); }
  }
  backendHost = server.substring(0, colon);
  backendHttpsPort = (uint16_t)server.substring(colon + 1).toInt();
  if (backendHttpsPort == 0) backendHttpsPort = BACKEND_HTTPS_PORT;
  deviceToken = token;
  Serial.printf("[CFG] server=%s:%u (HTTPS), UDP video port=%u\n",
                backendHost.c_str(), backendHttpsPort, BACKEND_UDP_PORT);
  Serial.printf("[CFG] token=%s (len=%d)\n", deviceToken.c_str(), deviceToken.length());

  // 2.7 派生 AES key
  deriveAesKey(token, aesKey);

  // 2.11 WiFi STA
  Serial.printf("[WIFI] connecting to %s\n", ssid.c_str());
  WiFi.mode(WIFI_STA);
  WiFi.begin(ssid.c_str(), password.c_str());
  uint32_t attemptStart = millis();
  while (WiFi.status() != WL_CONNECTED &&
         (millis() - attemptStart) < 30000) {
    delay(500);
    Serial.print(".");
  }
  if (WiFi.status() != WL_CONNECTED) {
    Serial.println("\n[WIFI] connect failed, rebooting in 5s");
    delay(5000);
    ESP.restart();
  }
  Serial.printf("\n[WIFI] connected, IP=%s, RSSI=%d dBm\n",
                WiFi.localIP().toString().c_str(), WiFi.RSSI());

  // 2.6 设备身份
  deviceId = generateDeviceId();
  Serial.printf("[DEV] deviceId=%s\n", deviceId.c_str());
  Serial.printf("[NET] backend HTTPS=%s:%u, UDP video=%s:%u\n",
                backendHost.c_str(), backendHttpsPort,
                backendHost.c_str(), BACKEND_UDP_PORT);

  // HTTPS 客户端：信任自签证书（开发期方案；生产应换 mTLS / CA pinning）
  httpsClient.setInsecure();
  httpsClient.setTimeout(POLL_HTTP_TIMEOUT_MS);
  Serial.println("[HTTPS] client ready (insecure mode for self-signed cert)");

  // 指令队列（pollTask → loop）
  cmdQueue = xQueueCreate(CMD_QUEUE_LEN, sizeof(String*));
  if (cmdQueue == NULL) {
    Serial.println("[BOOT] cmdQueue create failed, rebooting in 5s");
    delay(5000);
    ESP.restart();
  }

  // UDP 视频流发送（仅发送，绑定本地端口用于源端口稳定）
  udpSend.begin(BACKEND_UDP_PORT);

  // 帧序号随机起点（避免重启后 nonce 复用）
  frameSeq = esp_random();

  // 初始化 PID 状态
  memset(&pidState, 0, sizeof(pidState));

  // 2.11 发 register（HTTPS POST /api/device/{id}/register）
  sendRegister();

  // 启动视频采集任务（core 0，与 WiFi/BT 共享）
  xTaskCreatePinnedToCore(videoTask, "video", 8192, NULL, 1, &videoTaskHandle, 0);

  // 启动 HTTPS 长轮询任务（core 0，与 video 同核；网络 IO 不占 CPU）
  xTaskCreatePinnedToCore(pollTask, "poll", POLL_TASK_STACK, NULL, 1, &pollTaskHandle, 0);

  Serial.println("[BOOT] setup done");
}

void loop() {
  uint32_t now = millis();

  // 2.3 编码器 100ms 采样
  static uint32_t lastEncSample = 0;
  if (now - lastEncSample >= ENC_SAMPLE_MS) {
    lastEncSample = now;
    encoderSample();

    // 2.4 PID 周期消费：仅在 "W" 前进时运行
    if (targetDirection == "W" && targetPwm > 0) {
      int targetRpm = (int)((float)targetPwm / PWM_MAX * MAX_TARGET_RPM);

      // 2.5 命中缓存直接 setMotor
      int cachedPwm;
      if (pwmCacheLookup(targetRpm, cachedPwm)) {
        setMotor(cachedPwm, cachedPwm);
      } else {
        // PID 计算（纯函数：传入传出 state）
        uint32_t leftRpm  = getLeftRpm();
        uint32_t rightRpm = getRightRpm();
        PidState next;
        MotorPWM m = computePid(targetRpm, leftRpm, rightRpm, pidState, next);
        pidState = next;
        setMotor(m.left, m.right);

        // 收敛后写入缓存表
        if (pidState.convergeCount >= PID_CONVERGE_N) {
          int stablePwm = (m.left + m.right) / 2;
          pwmCacheStore(targetRpm, stablePwm);
          Serial.printf("[PID] cached target=%d rpm → pwm=%d\n", targetRpm, stablePwm);
        }
      }
    }
  }

  // 超时自动停止（durationMs）
  if (motionStopAt > 0 && now >= motionStopAt) {
    setMotor(0, 0);
    motionStopAt = 0;
    targetDirection = "stop";
    memset(&pidState, 0, sizeof(pidState));
    Serial.println("[MOTION] auto stop after durationMs");
  }

  // 2.8 派发 HTTPS 长轮询拉到的指令（pollTask → cmdQueue → handlers）
  dispatchCommands();

  // 2.10 运行时也支持 Web Serial 配网
  pollSerialConfig();

  // WiFi 断线重连
  if (WiFi.status() != WL_CONNECTED) {
    static uint32_t lastReconnect = 0;
    if (now - lastReconnect > 5000) {
      lastReconnect = now;
      Serial.println("[WIFI] disconnected, reconnecting...");
      WiFi.reconnect();
    }
  }
}
