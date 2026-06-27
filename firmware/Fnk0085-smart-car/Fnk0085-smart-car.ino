/* =================================================================
 * Fnk0085-smart-car.ino —— ESP32-S3 智能小车固件主入口
 * 版本：v0.3.1（与 backend/Cargo.toml、frontend/package.json 同步）
 * -----------------------------------------------------------------
 * 硬件平台：Freenove ESP32-S3 WROOM（带 PSRAM） + OV2640 + L298N + 槽型对射编码器
 * 功能模块：
 *   2.1  摄像头初始化（QVGA/JPEG/PSRAM，10fps，双缓冲 fb_count=2）
 *   2.2  L298N 双电机驱动（LEDC PWM，纯函数 setMotor）
 *   2.3  双编码器测速（中断计数 + 100ms 采样 → RPM）
 *   2.4  PID 软件平衡（纯函数 computePid 收敛后写缓存表）
 *   2.5  PWM 缓存表（命中跳过 PID）
 *   2.6  设备身份生成（chipId + MAC）
 *   2.7  HTTPS 视频帧上传（POST /api/device/{id}/frame，单帧 JPEG 二进制）
 *   2.8  HTTPS 控制通道：长轮询拉取指令 + POST 上报事件（WiFiClientSecure）
 *   2.9  拍照（UXGA + quality=4 写 SD）
 *   2.10 Web Serial 配网（CONFIG|... 行协议 + NVS，串口打印 token）
 *   2.11 WiFi STA + HTTP/HTTPS 双模式（控制平面 + 视频平面共用客户端，HTTP/1.1 keep-alive）
 *   2.12 函数式风格 + 中文注释
 * -----------------------------------------------------------------
 * 控制平面架构（HTTP/HTTPS 长轮询，scheme 由 server 字段决定）：
 *   - POST /api/device/{id}/register           注册（token 校验）
 *   - GET  /api/device/{id}/poll?timeout=30    长轮询拉指令（返回 cmd JSON 或 Ping）
 *   - POST /api/device/{id}/event               上报 photo_done/ack/error
 *   - POST /api/device/{id}/frame               单帧 JPEG 上传（视频平面）
 *   - TLS：setInsecure() 跳过证书校验（TLS 由 nginx 反代统一处理，设备侧不再固定 CA）
 *   - 双模式：server 字段含 https:// → useHttps=true 走 httpsClient；
 *             含 http://  → useHttps=false 走 plainClient（明文直连后端）；
 *             无 scheme 前缀（老配置）默认 useHttps=true
 *   - SNTP 时间同步：configTime(0, 0, "pool.ntp.org", "time.google.com")
 *   - pollTask（FreeRTOS，core 0）独占长轮询；指令通过 FreeRTOS 队列投递给 loop
 *   - videoTask（FreeRTOS，core 0）采集 + POST 单帧；与 pollTask 共享 httpsClient/plainClient
 *     （通过 httpsMutex 互斥访问，避免 HTTPClient 并发竞争）
 *   - loop（core 1）消费队列 + 100ms 编码器/PID 周期 + WiFi 重连
 *   - WiFi.config 显式指定 DNS1=119.29.29.29（DNSPod）+ DNS2=8.8.8.8（Google）
 *
 * 视频平面架构（HTTP/HTTPS POST 单帧）：
 *   采集 → esp_camera_fb_get() → httpsPostFrame() POST /api/device/{id}/frame
 *   → esp_camera_fb_return(fb) → vTaskDelay(100ms) 节奏控制 10fps
 *   POST 失败直接丢弃该帧（不重试），保证 10fps 节奏不塌
 * ================================================================= */

#include <Arduino.h>
#include <WiFi.h>
#include <WiFiClientSecure.h>
#include <HTTPClient.h>
#include <Preferences.h>
#include <ArduinoJson.h>
#include <SD_MMC.h>
#include <FS.h>
#include <esp_camera.h>
#include <esp_system.h>
#include <esp_efuse.h>
#include <esp_timer.h>
#include <time.h>
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
const uint16_t FRAME_POST_TIMEOUT_MS   = 2000; // 单帧 POST 2s 超时（10fps 节奏保护）

// 后端默认端口（NVS 未指定 server 时用作回退；正常流程由 CONFIG|server=host:port 注入）
const uint16_t BACKEND_HTTPS_PORT = 8080;  // HTTPS 端口（控制 + 视频共用）

// HTTPS 长轮询参数
const uint16_t POLL_TIMEOUT_S    = 30;    // 长轮询秒数（后端上限 60）
const uint16_t POLL_TASK_STACK   = 16384; // pollTask 栈字节
const uint16_t VIDEO_TASK_STACK  = 16384; // videoTask 栈字节（HTTPClient + TLS 握手栈占用大）
const uint16_t POLL_HTTP_TIMEOUT_MS = 35000; // HTTP 整体超时（略大于 poll 超时）
const uint16_t POLL_BACKOFF_MS   = 1000; // poll 失败后重试间隔

// SNTP 同步参数
const char*    SNTP_SERVER1     = "pool.ntp.org";
const char*    SNTP_SERVER2     = "time.google.com";
const uint32_t SNTP_TIMEOUT_MS  = 5000;   // SNTP 等待上限
const uint32_t SNTP_VALID_AFTER = 1700000000UL; // 2023 年后的合法时间戳

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

// 运动状态（由 control 指令设置，loop 周期消费）
String    targetDirection = "stop";
int       targetPwm       = 0;
uint32_t  motionStopAt    = 0;     // 0 表示不自动停止
PidState  pidState        = { 0.0f, 0.0f, 0.0f, 0.0f, 0 };

// 设备身份
String deviceId;
String deviceToken;
String backendHost;        // 后端 host（控制 + 视频共用，scheme 由 useHttps 决定）
uint16_t backendPort = BACKEND_HTTPS_PORT;  // 后端端口（默认 8080；http/https 共用）

// 网络：控制通道 + 视频帧上传复用同一 HTTP 客户端（双模式，HTTP/1.1 keep-alive）
// - useHttps=true（默认，兼容老配置）：走 httpsClient（WiFiClientSecure + setInsecure）
// - useHttps=false：走 plainClient（明文 WiFiClient，直连后端 http://）
// plainClient / httpsClient 共用 httpsMutex 互斥保护，避免 pollTask / videoTask / loop
// 三方并发竞争同一底层 TCP 连接（同一时刻只能由一个任务使用）
WiFiClientSecure    httpsClient;    // https 模式客户端（setInsecure，跳过证书校验）
WiFiClient           plainClient;    // http 模式客户端（明文，直连后端）
bool                 useHttps = true;  // scheme 开关；由 parseConfigLine 解析 server 字段决定
// TLS 握手失败标记（session 内 sticky）：
// - 首次 httpsClient 失败（http.POST/GET 返回 -1）后置 true，后续请求直接走 plainClient
// - 重启或 NVS 重配后复位为 false，重新尝试 TLS
bool                 httpsHandshakeFailed = false;
QueueHandle_t        cmdQueue = NULL;  // pollTask → loop 的指令队列
TaskHandle_t         pollTaskHandle = NULL;
SemaphoreHandle_t    httpsMutex = NULL;  // 保护 httpsClient 并发访问（pollTask / videoTask / loop 三方）

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

// HTTPS 控制通道
int  httpsPost(const String& path, const String& body, String& respOut);
int  httpsGet(const String& path, String& respOut);
int  httpsPostFrame(uint8_t* jpeg, size_t len, uint64_t uptimeMs);
int  probeHealth(bool useTls);
bool probeScheme();
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
  config.jpeg_quality = 5;     // OV2640：数值越低质量越高；5 ≈ 高质量单帧 15-30 KB
  config.fb_count     = 2;     // 双缓冲，降低采集抖动

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
 * 2.7 HTTPS 视频帧上传 + 控制通道
 * -----------------------------------------------------------------
 * - 双模式：useHttps=true 走 httpsClient（WiFiClientSecure + setInsecure，
 *   TLS 由 nginx 反代统一处理）；useHttps=false 走 plainClient（明文直连后端）。
 * - httpsPost / httpsGet / httpsPostFrame 根据 useHttps 选择客户端；同一时刻
 *   只允许一个任务进入临界区（httpsMutex 互斥），避免底层 TCP 连接被并发
 *   复用竞争（pollTask / videoTask / loop 三方共享）。
 * - 返回 HTTP 状态码，<0 表示网络错误。
 * ================================================================= */

// 构造完整 URL：scheme 由 useHttps 决定（http:// 或 https://）
String buildUrl(const String& path) {
  String scheme = useHttps ? "https://" : "http://";
  return scheme + backendHost + ":" + String(backendPort) + path;
}

// 互斥锁辅助 RAII：进入临界区持锁，离开自动释放
// 取不到锁时返回 false，调用方应跳过本次操作（避免阻塞 videoTask 节奏）
bool httpsLockTake(uint32_t timeoutMs) {
  if (httpsMutex == NULL) return false;
  return xSemaphoreTake(httpsMutex, pdMS_TO_TICKS(timeoutMs)) == pdPASS;
}
void httpsLockGive() {
  if (httpsMutex != NULL) xSemaphoreGive(httpsMutex);
}

// 打印 TLS 与 HTTPClient 错误诊断日志（HTTPS 模式且未回退时取 TLS 错误码）
// 调用方传入 HTTPClient 引用、返回 code、tag（如 "POST"/"GET"/"FRAME"）
void logTlsAndHttpError(HTTPClient& http, int code, const char* tag) {
  if (useHttps && !httpsHandshakeFailed) {
    char errBuf[128] = {0};
    int tlsErr = httpsClient.lastError(errBuf, sizeof(errBuf));
    Serial.printf("[TLS] %s handshake failed: code=%d %s (backend may be plain HTTP)\n", tag, tlsErr, errBuf);
  }
  Serial.printf("[HTTP] %s error=%d %s\n", tag, code, http.errorToString(code).c_str());
}

// 探测后端 /api/health 端点可用性（启动时 scheme 检测，2s 超时）
// useTls=true 用 httpsClient，false 用 plainClient；返回 HTTP 状态码（200 成功，<0 网络/握手错误）
// 静默执行（不调用 logTlsAndHttpError），失败时仅返回 code 由调用方决策
int probeHealth(bool useTls) {
  HTTPClient http;
  String url = (useTls ? "https://" : "http://") + backendHost + ":" + String(backendPort) + "/api/health";
  bool ok = useTls ? http.begin(httpsClient, url) : http.begin(plainClient, url);
  if (!ok) {
    Serial.printf("[NET] probe begin failed: %s\n", url.c_str());
    return -1;
  }
  http.setTimeout(2000);
  int code = http.GET();
  http.end();
  return code;
}

// 启动时主动探测后端 scheme：先试 NVS 配的 scheme，失败则试另一 scheme
// - NVS scheme ok：保持原 scheme，返回 true
// - NVS scheme 失败 + 另一 scheme ok：切换 useHttps、重置 httpsHandshakeFailed、写回 NVS，返回 true
// - 两个都失败：打印总结日志，保留原 scheme，返回 false（依赖 https-fallback-and-diagnostics 兜底）
bool probeScheme() {
  bool nvSchemeHttps = useHttps;
  int code1 = probeHealth(nvSchemeHttps);
  if (code1 == 200) {
    Serial.printf("[NET] probe: %s ok, using %s\n",
                  nvSchemeHttps ? "https" : "http",
                  nvSchemeHttps ? "https" : "http");
    return true;
  }
  int code2 = probeHealth(!nvSchemeHttps);
  if (code2 == 200) {
    useHttps = !nvSchemeHttps;
    httpsHandshakeFailed = false;
    String newServer = (useHttps ? "https://" : "http://") + backendHost + ":" + String(backendPort);
    Preferences prefs;
    prefs.begin(NVS_NAMESPACE, false);
    prefs.putString("server", newServer);
    prefs.end();
    Serial.printf("[NET] probe: %s failed (code=%d), %s ok, switching to %s (NVS updated)\n",
                  nvSchemeHttps ? "https" : "http", code1,
                  useHttps ? "https" : "http",
                  useHttps ? "https" : "http");
    return true;
  }
  Serial.printf("[NET] probe failed: http=%d, https=%d, fallback to NVS scheme=%s\n",
                nvSchemeHttps ? code2 : code1,
                nvSchemeHttps ? code1 : code2,
                nvSchemeHttps ? "https" : "http");
  return false;
}

// POST JSON 到指定 path；返回 HTTP 状态码（>=200），<0 为网络/协议错误
// 调用前后持 httpsMutex 互斥保护客户端（pollTask / videoTask / loop 三方并发）
// 根据 useHttps 选择 httpsClient（TLS）或 plainClient（明文）
int httpsPost(const String& path, const String& body, String& respOut) {
  if (!httpsLockTake(POLL_HTTP_TIMEOUT_MS)) {
    Serial.println("[HTTPS] POST lock timeout");
    return -1;
  }
  HTTPClient http;
  String url = buildUrl(path);
  bool useTls = useHttps && !httpsHandshakeFailed;
  bool ok = useTls ? http.begin(httpsClient, url)
                   : http.begin(plainClient, url);
  if (!ok) {
    Serial.printf("[HTTPS] POST begin failed: %s\n", url.c_str());
    httpsLockGive();
    return -1;
  }
  http.addHeader("Content-Type", "application/json");
  http.addHeader("Authorization", "Bearer " + deviceToken);
  http.setTimeout(POLL_HTTP_TIMEOUT_MS);
  int code = http.POST(body);
  if (code == -1 && useTls) {
    // TLS 握手失败（如对端为明文 HTTP 端口）：打印诊断日志、置 sticky 标记、回退 plainClient 重试一次
    logTlsAndHttpError(http, code, "POST");
    httpsHandshakeFailed = true;
    http.end();
    HTTPClient http2;
    if (!http2.begin(plainClient, url)) {
      Serial.printf("[HTTPS] POST retry begin failed: %s\n", url.c_str());
      httpsLockGive();
      return -1;
    }
    http2.addHeader("Content-Type", "application/json");
    http2.addHeader("Authorization", "Bearer " + deviceToken);
    http2.setTimeout(POLL_HTTP_TIMEOUT_MS);
    int retryCode = http2.POST(body);
    if (retryCode > 0) respOut = http2.getString();
    http2.end();
    httpsLockGive();
    return retryCode;
  }
  if (code < 0) {
    // 其它网络/协议错误：仅打印日志，不回退
    logTlsAndHttpError(http, code, "POST");
  } else if (code > 0) {
    respOut = http.getString();
  }
  http.end();
  httpsLockGive();
  return code;
}

// GET 指定 path（长轮询）；返回 HTTP 状态码，<0 为网络/协议错误
int httpsGet(const String& path, String& respOut) {
  if (!httpsLockTake(POLL_HTTP_TIMEOUT_MS)) {
    Serial.println("[HTTPS] GET lock timeout");
    return -1;
  }
  HTTPClient http;
  String url = buildUrl(path);
  bool useTls = useHttps && !httpsHandshakeFailed;
  bool ok = useTls ? http.begin(httpsClient, url)
                   : http.begin(plainClient, url);
  if (!ok) {
    Serial.printf("[HTTPS] GET begin failed: %s\n", url.c_str());
    httpsLockGive();
    return -1;
  }
  http.addHeader("Authorization", "Bearer " + deviceToken);
  http.addHeader("Cache-Control", "no-store");
  http.setTimeout(POLL_HTTP_TIMEOUT_MS);
  int code = http.GET();
  if (code == -1 && useTls) {
    // TLS 握手失败：打印诊断日志、置 sticky 标记、回退 plainClient 重试一次
    logTlsAndHttpError(http, code, "GET");
    httpsHandshakeFailed = true;
    http.end();
    HTTPClient http2;
    if (!http2.begin(plainClient, url)) {
      Serial.printf("[HTTPS] GET retry begin failed: %s\n", url.c_str());
      httpsLockGive();
      return -1;
    }
    http2.addHeader("Authorization", "Bearer " + deviceToken);
    http2.addHeader("Cache-Control", "no-store");
    http2.setTimeout(POLL_HTTP_TIMEOUT_MS);
    int retryCode = http2.GET();
    if (retryCode > 0) respOut = http2.getString();
    http2.end();
    httpsLockGive();
    return retryCode;
  }
  if (code < 0) {
    // 其它网络/协议错误：仅打印日志，不回退
    logTlsAndHttpError(http, code, "GET");
  } else if (code > 0) {
    respOut = http.getString();
  }
  http.end();
  httpsLockGive();
  return code;
}

// POST 单帧 JPEG 到 /api/device/{id}/frame；返回 HTTP 状态码，<0 为网络错误
// body 为原始 JPEG 二进制；header 携带 token + uptime（用于前端延时测量）
// 单帧超时 2s（FRAME_POST_TIMEOUT_MS），保证 10fps 节奏不塌
int httpsPostFrame(uint8_t* jpeg, size_t len, uint64_t uptimeMs) {
  if (!httpsLockTake(FRAME_POST_TIMEOUT_MS)) {
    Serial.println("[FRAME] lock timeout");
    return -1;
  }
  String url = buildUrl("/api/device/" + deviceId + "/frame");
  HTTPClient http;
  bool useTls = useHttps && !httpsHandshakeFailed;
  bool ok = useTls ? http.begin(httpsClient, url)
                   : http.begin(plainClient, url);
  if (!ok) {
    log_e("[FRAME] begin 失败");
    httpsLockGive();
    return -1;
  }
  http.addHeader("Authorization", "Bearer " + deviceToken);
  http.addHeader("Content-Type", "image/jpeg");
  http.addHeader("X-Device-Uptime-Ms", String((unsigned long long)uptimeMs));
  http.setTimeout(FRAME_POST_TIMEOUT_MS);
  int code = http.POST(jpeg, len);
  if (code == -1 && useTls) {
    // TLS 握手失败：打印诊断日志、置 sticky 标记、回退 plainClient 重试一次
    logTlsAndHttpError(http, code, "FRAME");
    httpsHandshakeFailed = true;
    http.end();
    HTTPClient http2;
    if (!http2.begin(plainClient, url)) {
      log_e("[FRAME] retry begin 失败");
      httpsLockGive();
      return -1;
    }
    http2.addHeader("Authorization", "Bearer " + deviceToken);
    http2.addHeader("Content-Type", "image/jpeg");
    http2.addHeader("X-Device-Uptime-Ms", String((unsigned long long)uptimeMs));
    http2.setTimeout(FRAME_POST_TIMEOUT_MS);
    int retryCode = http2.POST(jpeg, len);
    http2.end();
    httpsLockGive();
    if (retryCode != 200 && retryCode != 204) {
      log_e("[FRAME] POST 失败 code=%d len=%u", retryCode, (unsigned)len);
    }
    return retryCode;
  }
  if (code < 0) {
    // 其它网络/协议错误：仅打印日志，不回退
    logTlsAndHttpError(http, code, "FRAME");
  }
  http.end();
  httpsLockGive();
  if (code != 200 && code != 204) {
    log_e("[FRAME] POST 失败 code=%d len=%u", code, (unsigned)len);
  }
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
  Serial.println("[TASK] poll started");
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
 * 流程：esp_camera_fb_get() → httpsPostFrame() → esp_camera_fb_return()
 * 节奏控制：vTaskDelayUntil 保证 10fps；POST 失败直接丢帧（不重试）
 * 拍照互斥：photoInProgress 期间跳过采集（与 vTaskSuspend 双重保护）
 * ================================================================= */
void videoTask(void* arg) {
  Serial.println("[TASK] video started");
  uint32_t frameOk = 0;
  uint32_t frameFail = 0;
  TickType_t lastWake = xTaskGetTickCount();
  for (;;) {
    vTaskDelayUntil(&lastWake, pdMS_TO_TICKS(VIDEO_FRAME_INTERVAL_MS));
    // 拍照进行中跳过（双重保护：vTaskSuspend + 此标志）
    if (photoInProgress) continue;

    camera_fb_t* fb = esp_camera_fb_get();
    if (!fb) {
      vTaskDelay(pdMS_TO_TICKS(50));
      continue;
    }
    uint64_t uptime = (uint64_t)(esp_timer_get_time() / 1000);
    int code = httpsPostFrame(fb->buf, fb->len, uptime);
    if (code == 200 || code == 204) {
      frameOk++;
    } else {
      frameFail++;
    }
    esp_camera_fb_return(fb);

    // 每 10 帧汇总一次状态
    uint32_t total = frameOk + frameFail;
    if (total > 0 && total % 10 == 0) {
      log_i("[VIDEO] ok=%u fail=%u", (unsigned)frameOk, (unsigned)frameFail);
    }
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

  // 5. 恢复 QVGA + quality=5（与 cameraInit 一致，避免帧质量降级）
  if (s) {
    s->set_framesize(s, FRAMESIZE_QVGA);
    s->set_quality(s, 5);
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

// 剥离 server 字段中的 scheme 前缀（http:// 或 https://）
// - 返回剥离 scheme 后的 host[:port] 字符串（原样保留 host:port 部分）
// - 通过 outUseHttps 输出 scheme；无 scheme 前缀时默认 true（兼容老配置）
String stripServerScheme(const String& server, bool& outUseHttps) {
  if (server.startsWith("https://")) {
    outUseHttps = true;
    return server.substring(8);   // 跳过 "https://"
  }
  if (server.startsWith("http://")) {
    outUseHttps = false;
    return server.substring(7);   // 跳过 "http://"
  }
  // 兼容老配置：无 scheme 前缀 → 默认走 HTTPS（与 v0.2.x 行为一致）
  outUseHttps = true;
  return server;
}

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
  // NVS 重配后复位 TLS 回退标记，让设备用新 server 重新尝试 TLS
  httpsHandshakeFailed = false;
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
          // 剥离 scheme 前缀后再校验 host:port（兼容 http:// https:// 与无 scheme）
          bool schemeHttps = true;
          String hostPort = stripServerScheme(server, schemeHttps);
          int colon = hostPort.indexOf(':');
          if (colon <= 0) {
            Serial.println("ERR|invalid_server (missing :port)");
          } else {
            long port = hostPort.substring(colon + 1).toInt();
            if (port <= 0 || port > 65535) {
              Serial.println("ERR|invalid_server (port out of range)");
            } else {
              // 写 NVS（server 原样保留 scheme 前缀，便于启动时识别 scheme）
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

  // 剥离 scheme 前缀；解析 host:port（默认 8080）
  // - https:// → useHttps=true（走 httpsClient + setInsecure）
  // - http://  → useHttps=false（走 plainClient 明文直连）
  // - 无 scheme 前缀（老配置）→ 默认 useHttps=true，原样保留 server
  bool schemeHttps = true;
  String hostPort = stripServerScheme(server, schemeHttps);
  useHttps = schemeHttps;
  int colon = hostPort.indexOf(':');
  if (colon <= 0) {
    Serial.println("[CFG] invalid server in NVS, waiting for re-provisioning");
    while (true) { pollSerialConfig(); delay(10); }
  }
  backendHost = hostPort.substring(0, colon);
  backendPort = (uint16_t)hostPort.substring(colon + 1).toInt();
  if (backendPort == 0) backendPort = BACKEND_HTTPS_PORT;
  deviceToken = token;
  Serial.printf("[Config] scheme=%s host=%s port=%u\n",
                useHttps ? "https" : "http",
                backendHost.c_str(), backendPort);
  Serial.printf("[CFG] token=%s (len=%d)\n", deviceToken.c_str(), deviceToken.length());

  // 2.11 WiFi STA
  Serial.printf("[WIFI] connecting to %s\n", ssid.c_str());
  WiFi.mode(WIFI_STA);
  // WiFi.config：5 参数版（local_ip + gateway + subnet + dns1 + dns2）
  // 前 3 项设 INADDR_NONE 表示由 DHCP 自动获取 IP/Gateway/Subnet，
  // 仅显式指定 DNS1=119.29.29.29（DNSPod）+ DNS2=8.8.8.8（Google），
  // 避免路由器 DNS 解析失败导致连不上后端域名
  WiFi.config(INADDR_NONE, INADDR_NONE, INADDR_NONE,
              IPAddress(119, 29, 29, 29), IPAddress(8, 8, 8, 8));
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
  Serial.printf("[WiFi] DNS1=%s DNS2=%s\n",
                WiFi.dnsIP(0).toString().c_str(),
                WiFi.dnsIP(1).toString().c_str());

  // SNTP 时间同步（WiFi 连接成功后；mbedTLS 证书时间校验依赖）
  // 失败仅告警不阻塞（后端证书 not_before=1970 / not_after=2099 兜底）
  Serial.println("[SNTP] syncing time...");
  configTime(0, 0, SNTP_SERVER1, SNTP_SERVER2);
  uint32_t sntpStart = millis();
  time_t now = 0;
  bool sntpOk = false;
  while ((millis() - sntpStart) < SNTP_TIMEOUT_MS) {
    now = time(nullptr);
    if ((uint32_t)now > SNTP_VALID_AFTER) {
      sntpOk = true;
      break;
    }
    delay(200);
  }
  if (sntpOk) {
    Serial.printf("[SNTP] 同步成功 %lu\n", (unsigned long)now);
  } else {
    Serial.println("[SNTP] 同步失败，依赖证书宽限期（1970-2099）");
  }

  // 2.6 设备身份
  deviceId = generateDeviceId();
  Serial.printf("[DEV] deviceId=%s\n", deviceId.c_str());
  Serial.printf("[NET] backend %s://%s:%u (useHttps=%d, fallbackReady=%d)\n",
                useHttps ? "https" : "http",
                backendHost.c_str(), backendPort,
                useHttps ? 1 : 0,
                httpsHandshakeFailed ? 0 : 1);

  // HTTPS 客户端：setInsecure() 跳过证书校验（TLS 由 nginx 反代统一处理；
  // 设备侧不再固定 CA，同时支持 http:// 直连后端与 https:// 经 nginx 双模式）
  httpsClient.setInsecure();
  httpsClient.setTimeout(POLL_HTTP_TIMEOUT_MS);
  Serial.println("[HTTPS] client ready (setInsecure mode)");

  // 启动时主动探测后端 scheme（避免冷启动刷 TLS 错误日志）；探测失败不阻塞 register
  probeScheme();

  // 指令队列（pollTask → loop）
  cmdQueue = xQueueCreate(CMD_QUEUE_LEN, sizeof(String*));
  if (cmdQueue == NULL) {
    Serial.println("[BOOT] cmdQueue create failed, rebooting in 5s");
    delay(5000);
    ESP.restart();
  }

  // httpsClient 互斥锁（pollTask / videoTask / loop 三方并发访问保护）
  httpsMutex = xSemaphoreCreateMutex();
  if (httpsMutex == NULL) {
    Serial.println("[BOOT] httpsMutex create failed, rebooting in 5s");
    delay(5000);
    ESP.restart();
  }

  // 初始化 PID 状态
  memset(&pidState, 0, sizeof(pidState));

  // 2.11 发 register（HTTPS POST /api/device/{id}/register）
  sendRegister();

  // 启动视频采集任务（core 0，与 WiFi/BT 共享）
  xTaskCreatePinnedToCore(videoTask, "video", VIDEO_TASK_STACK, NULL, 1, &videoTaskHandle, 0);

  // 启动 HTTPS 长轮询任务（core 0，与 video 同核；网络 IO 不占 CPU）
  xTaskCreatePinnedToCore(pollTask, "poll", POLL_TASK_STACK, NULL, 1, &pollTaskHandle, 0);

  Serial.println("[BOOT] setup done");

  // 任务句柄校验：创建失败则延时后软重启，避免后续空指针访问
  if (videoTaskHandle == NULL || pollTaskHandle == NULL) {
    Serial.println("[BOOT] task create failed, rebooting in 5s");
    delay(5000);
    ESP.restart();
  }
  Serial.println("[BOOT] tasks launched");
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
