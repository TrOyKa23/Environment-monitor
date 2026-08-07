# Environment-monitor

A smart environmental monitoring station powered by Raspberry Pi Pico 2 W with SD logging, a real-time LCD dashboard, and Wi-Fi data sync for long-term analytics.

:::info

**Author**: TrOyKa23 \
**GitHub Project Link**: https://github.com/TrOyKa23/Environment-monitor

:::

## Description

Environment-monitor is an embedded Rust application built on the Embassy async framework for the Raspberry Pi Pico 2 W (RP2350). It periodically samples ambient temperature and barometric pressure using a BME280 sensor over I2C, displays real-time sparkline graphs and telemetry on an ST7789 LCD screen, logs data to a MicroSD card in CSV format (`TEMPLOG.CSV`), and transmits batch logs every 30 minutes via Wi-Fi to a remote server for daily, weekly, monthly, and yearly trend plotting.

## Motivation

This project was built for educational and self-learning purposes to explore embedded development in Rust, modern async hardware runtimes (Embassy), shared SPI bus management, filesystem operations on microcontrollers, and IoT network protocol stacks.

## Architecture

The architecture consists of an async Rust runtime executing concurrent tasks on the RP2350 microcontroller:

- **Main Processor**: Raspberry Pi Pico 2 W running the Embassy async executor.
- **BME280 Sensor**: Communicates over I2C0 to provide temperature and pressure readings.
- **ST7789 Display & SD Card**: Share the SPI1 bus using mutex-guarded SPI device handles (`SpiDeviceWithConfig`).
- **SWD Probe**: A secondary Raspberry Pi Pico 2 W acts as an external hardware debugger via SWD (SWCLK, SWDIO).
- **Wi-Fi Subsystem**: Uses `cyw43` and `embassy-net` stacks to handle TCP/IP network transport.

```
                      +-------------------------+           +-------------------------+
                      |            CORE         |           |            DEBUG        |
                      |   Raspberry Pi Pico 2 W |-----------|   Raspberry Pi Pico 2 W |
                      |        (RP2350)         |           |        (RP2350)         |
                      +------------+------------+           +------------+------------+
                                   |
         +-------------------------+-------------------------+
         | I2C0                    | SPI1 (Shared Bus)       | CYW43439 (Wi-Fi)
         v                         |                         v
+------------------+     +---------+---------+      +------------------+
| BME280 Sensor    |     |                   |      | Server / Web App |
| Temp & Pressure  |     v                   v      | (Trend Graphs)   |
+------------------+ +-------+           +-------+  +------------------+
                     | ST7789|           | SD    |
                     | LCD   |           | Card  |
                     +-------+           +-------+
```

## Log

### Milestone 1 — Project Initialization & Sensor Setup

- Configured Rust toolchain for target `thumbv8m.main-none-eabihf` and RP2350 chip definitions (`memory.x`, `build.rs`).
- Integrated `bme280-rs` driver over async I2C.
- Set up RTT logging via `defmt-rtt` and `panic-probe`.
- Soldered pins on raspberry pi pico 2w and on the bmp280 sensor

![Milestone 1](./1.webp)
![Milestone 1](./2.webp)

### Milestone 2 — Shared SPI Bus & Display UI

- Implemented ST7789 display driver using `mipidsi` over SPI1.
- Created `ui.rs` dashboard featuring a top header bar, status icons, uptime counter, big numerical readouts, and custom sparkline history charts using `embedded-graphics`.

![Milestone 2](./3.webp)

### Milestone 3 — SD Card Filesystem Integration

- Implemented shared SPI bus architecture (`SpiDeviceWithConfig` and `NoopRawMutex`) to allow safe multiplexing between display and SD card.
- Integrated `embedded-sdmmc` to manage FAT volumes and automate CSV log writing (`TEMPLOG.CSV`) with formatting and header initialization.
- Syncronizing UI and SD data

![Milestone 2](./4.webp)

### Milestone 4 — Async Network Stack & Server Sync

- Configured PIO driver (`cyw43-pio`) and background tasks for the CYW43439 wireless chip.
- Implemented 30-minute interval timer to packetize stored logs and push them over TCP/HTTP to a server for web-based graph rendering.

## Hardware

The system utilizes a Raspberry Pi Pico 2 W as the central unit connected to an environmental sensor, a color TFT display, and an integrated MicroSD card module.

### Schematics

![Hardware Schematic](./schema.webp)

### Bill of Materials

| Device                                                                                                                                                                                                 | Usage                                     | Price  |
| ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------- | ------ |
| [Raspberry Pi Pico 2 W](https://www.emag.ro/modul-microcontroler-raspberry-pi-pico-2-w-rp2350-520-kb-on-chip-sram-51x21mm-pico2w/pd/DGXG5M3BM/?ref=history-shopping_481466799_10095_1)                 | Core Microcontroller (RP2350) + Wi-Fi     | 60 RON |
| [Raspberry Pi Pico 2 W](https://www.emag.ro/modul-microcontroler-raspberry-pi-pico-2-w-rp2350-520-kb-on-chip-sram-51x21mm-pico2w/pd/DGXG5M3BM/?ref=history-shopping_481466799_10095_1)                 | Secondary Pico used as SWD Debugger Probe | 60 RON |
| [BME280 Sensor Module](https://www.emag.ro/modul-senzor-temperatura-umiditate-presiune-bme280-ai0002-s34/pd/DR7HCZBBM/?ref=history-shopping_495609421_257829_1)                                        | Temperature and Pressure sensor (I2C)     | 22 RON |
| [ST7789 2.4" TFT LCD Module with SD Slot](https://www.emag.ro/display-tft-spi-2-4-inch-240x320-lcd-cu-touchscreen-driver-st7789v-arduino-emg178/pd/DXZMBSYBM/?ref=history-shopping_482724113_221614_1) | 240x320 Display + SD Card Reader (SPI)    | 50 RON |

## Software

| Library                                                         | Description                       | Usage                                                       |
| --------------------------------------------------------------- | --------------------------------- | ----------------------------------------------------------- |
| [embassy-executor](https://github.com/embassy-rs/embassy)       | Async task executor               | Drives background execution tasks                           |
| [embassy-rp](https://github.com/embassy-rs/embassy)             | RP2350 Hardware Abstraction Layer | Manages I2C, SPI, GPIO, PIO, and hardware peripherals       |
| [bme280-rs](https://crates.io/crates/bme280-rs)                 | Async BME280 sensor driver        | Reads temperature and pressure data asynchronously          |
| [mipidsi](https://crates.io/crates/mipidsi)                     | Display controller driver         | Drives ST7789 LCD display initialization                    |
| [embedded-graphics](https://crates.io/crates/embedded-graphics) | 2D graphics engine                | Renders text, icons, containers, and sparkline trend graphs |
| [embedded-sdmmc](https://crates.io/crates/embedded-sdmmc)       | FAT volume and SD card driver     | Writes CSV log records to SD filesystem                     |
| [cyw43](https://crates.io/crates/cyw43)                         | Wi-Fi chip driver                 | Manages wireless connection on CYW43439                     |
| [embassy-net](https://crates.io/crates/embassy-net)             | Async network stack               | Handles DHCP, TCP, and network sockets                      |
| [defmt](https://crates.io/crates/defmt)                         | Efficient logging framework       | Prints internal diagnostics and status over SWD/RTT         |

## Links

1. [Raspberry Pi Pico 2 W Documentation](https://www.raspberrypi.com/documentation/microcontrollers/pico-series.html)
2. [Embassy Async Framework Documentation](https://embassy.dev/)
3. [RP2350 Datasheet](https://datasheets.raspberrypi.com/rp2350/rp2350-datasheet.pdf)
4. [Project Repository](https://github.com/TrOyKa23/Environment-monitor)
