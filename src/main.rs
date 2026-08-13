#![no_std]
#![no_main]

mod bme_sensor;
mod display;
mod netsync;
mod rtc;
mod sdcard;
mod ui;

use core::cell::RefCell;
use defmt::info;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::i2c::{self, Config as I2cConfig, InterruptHandler as I2cInterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_rp::spi::{self, Spi};
use embassy_sync::blocking_mutex::Mutex;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_time::{Delay, Instant, Timer};

use {defmt_rtt as _, panic_probe as _};

const WIFI_SSID: &str = "---";
const WIFI_PASSWORD: &str = "----";

// Обязательный заголовок образа для RP2350 (аналог boot2 у RP2040)
#[unsafe(link_section = ".start_block")]
#[used]
static IMAGE_DEF: embassy_rp::block::ImageDef = embassy_rp::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
});

#[embassy_executor::task]
async fn wifi_sync_task(spawner: Spawner, pins: netsync::WifiPins) {
    let _stack = netsync::start(spawner, pins, WIFI_SSID, WIFI_PASSWORD).await;
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    let sda = p.PIN_4;
    let scl = p.PIN_5;
    let i2c = i2c::I2c::new_async(p.I2C0, scl, sda, Irqs, I2cConfig::default());

    let mut bme280 = match bme_sensor::init(i2c, Delay).await {
        Ok(sensor) => sensor,
        Err(_) => {
            info!("BME280 init failed - halting");
            loop {
                Timer::after_secs(1).await;
            }
        }
    };

    // TIP 3
    let clk = p.PIN_10;
    let mosi = p.PIN_11;
    let miso = p.PIN_8;
    let dc = Output::new(p.PIN_12, Level::Low);
    let mut rst = Output::new(p.PIN_15, Level::High);
    let lcd_cs = Output::new(p.PIN_13, Level::High);
    let sd_cs = Output::new(p.PIN_9, Level::High);

    let mut lcd_config = spi::Config::default();
    lcd_config.frequency = 32_000_000;

    let mut sd_config = spi::Config::default();
    sd_config.frequency = 400_000;

    let spi_bus = Spi::new_blocking(p.SPI1, clk, mosi, miso, lcd_config.clone());
    let spi_bus: Mutex<NoopRawMutex, _> = Mutex::new(RefCell::new(spi_bus));

    let lcd_spi = SpiDeviceWithConfig::new(&spi_bus, lcd_cs, lcd_config);
    let sd_spi = SpiDeviceWithConfig::new(&spi_bus, sd_cs, sd_config);

    rst.set_low();
    Timer::after_millis(10).await;
    rst.set_high();
    Timer::after_millis(10).await;

    // SD-карта ----------
    let logger = sdcard::TempLogger::new(sd_spi);

    // LCD ----------
    let mut lcd_delay = Delay;
    let mut buffer = [0u8; 512];
    let mut disp = display::init(lcd_spi, dc, &mut buffer, &mut lcd_delay);

    let mut dashboard = ui::Dashboard::new();

    info!("Display initialized");

    // wifi sync in background
    let wifi_pins = netsync::WifiPins {
        pwr: p.PIN_23,
        cs: p.PIN_25,
        pio: p.PIO0,
        dio: p.PIN_24,
        clk: p.PIN_29,
        dma: p.DMA_CH0,
    };
    spawner.spawn(wifi_sync_task(spawner, wifi_pins).unwrap());

    let start = Instant::now();

    loop {
        match bme280.read_sample().await {
            Ok(sample) => {
                let temp_c = sample.temperature.unwrap_or(0.0);
                let pressure_hpa = sample.pressure.unwrap_or(0.0) / 100.0;

                let t_int = temp_c as i32;
                let t_frac = ((temp_c * 10.0) as i32 % 10).abs();
                let p_int = pressure_hpa as i32;
                let p_frac = ((pressure_hpa * 10.0) as i32 % 10).abs();

                info!(
                    "temp: {}.{} C, pressure: {}.{} hPa",
                    t_int, t_frac, p_int, p_frac
                );

                let uptime_secs = (Instant::now() - start).as_secs();

                dashboard.update(&mut disp, uptime_secs, temp_c, pressure_hpa);

                if let Some(logger) = &logger {
                    logger.log_sample(temp_c, pressure_hpa);
                }
            }
            Err(_e) => info!("Error reading sample"),
        }

        Timer::after_secs(1).await;
    }
}
