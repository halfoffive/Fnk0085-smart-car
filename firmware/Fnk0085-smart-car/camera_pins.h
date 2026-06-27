/*
 * camera_pins.h
 * 复制自 Freenove ESP32-S3 WROOM Board 示例 Sketch_07.2_As_VideoWebServer。
 * 仅保留 CAMERA_MODEL_ESP32S3_EYE 相关定义（本项目的目标板）。
 *
 * 注意：CAMERA_MODEL_ESP32S3_EYE 的引脚分配与本项目 L298N/编码器引脚存在
 * 物理冲突（GPIO4/5/6/7 同时被摄像头 SIOD/SIOC/VSYNC/HREF 占用，GPIO15
 * 同时被摄像头 XCLK 与右编码器占用）。代码按 task 描述照常实现，硬件设计
 * 层面的引脚冲突由硬件工程师在 PCB 上规避（例如通过 GPIO 扩展或更换编码器引脚）。
 */
#ifndef CAMERA_PINS_H_
#define CAMERA_PINS_H_

#if defined(CAMERA_MODEL_ESP32S3_EYE)
#define PWDN_GPIO_NUM    -1
#define RESET_GPIO_NUM   -1
#define XCLK_GPIO_NUM    15
#define SIOD_GPIO_NUM    4
#define SIOC_GPIO_NUM    5

#define Y2_GPIO_NUM      11
#define Y3_GPIO_NUM      9
#define Y4_GPIO_NUM      8
#define Y5_GPIO_NUM      10
#define Y6_GPIO_NUM      12
#define Y7_GPIO_NUM      18
#define Y8_GPIO_NUM      17
#define Y9_GPIO_NUM      16

#define VSYNC_GPIO_NUM   6
#define HREF_GPIO_NUM    7
#define PCLK_GPIO_NUM    13
#else
#error "Camera model not selected, please #define CAMERA_MODEL_ESP32S3_EYE before including camera_pins.h"
#endif

#endif /* CAMERA_PINS_H_ */
