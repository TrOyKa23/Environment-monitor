#![no_std]
#![no_main]

use bme280_rs::{AsyncBme280, Configuration, Oversampling, SensorMode};
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
use embedded_graphics::mono_font::MonoTextStyle;
use embedded_graphics::mono_font::ascii::FONT_10X20;
use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::{PrimitiveStyle, Rectangle};
use embedded_graphics::text::Text;
use embedded_sdmmc::{Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use heapless::String;
use mipidsi::Builder;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation, Rotation};

use {defmt_rtt as _, panic_probe as _};

// Обязательный заголовок образа для RP2350 (аналог boot2 у RP2040)
#[unsafe(link_section = ".start_block")]
#[used]
static IMAGE_DEF: embassy_rp::block::ImageDef = embassy_rp::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
});

const LOG_FILE: &str = "TEMPLOG.CSV";

/// TIP 1 --------
#[derive(Default)]
struct DummyTimesource;

impl TimeSource for DummyTimesource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

/// TIP 2 --------
fn format_uptime(buf: &mut String<16>, elapsed_secs: u64) {
    let h = elapsed_secs / 3600;
    let m = (elapsed_secs % 3600) / 60;
    let s = elapsed_secs % 60;
    let _ = core::fmt::write(buf, format_args!("{:02}:{:02}:{:02}", h, m, s));
}

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // BME280 (I2C0) --------
    let sda = p.PIN_4;
    let scl = p.PIN_5;
    let i2c = i2c::I2c::new_async(p.I2C0, scl, sda, Irqs, I2cConfig::default());

    info!("Initializing BME280...");
    let mut bme280 = AsyncBme280::new(i2c, Delay);

    match bme280.init().await {
        Ok(_) => info!("BME280 initialized"),
        Err(e) => {
            info!("Init error: {:?}", defmt::Debug2Format(&e));
            loop {
                Timer::after_secs(1).await;
            }
        }
    }

    match bme280
        .set_sampling_configuration(
            Configuration::default()
                .with_temperature_oversampling(Oversampling::Oversample1)
                .with_pressure_oversampling(Oversampling::Oversample1)
                .with_humidity_oversampling(Oversampling::Oversample1)
                .with_sensor_mode(SensorMode::Normal),
        )
        .await
    {
        Ok(_) => info!("Config done"),
        Err(_) => info!("Error config"),
    }

    // TIP 3 --------
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

    // SD --------
    let sdcard = SdCard::new(sd_spi, Delay);
    let card_ok = match sdcard.num_bytes() {
        Ok(size) => {
            info!("SD card size: {} bytes", size);
            true
        }
        Err(_) => {
            info!("SD card init failed - logging to SD disabled");
            false
        }
    };

    let volume_mgr = VolumeManager::new(sdcard, DummyTimesource);

    if card_ok {
        match volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(volume0) => match volume0.open_root_dir() {
                Ok(root_dir) => {
                    // table opening try: if OK - file (table) exists and header doesn't need to be written
                    let file_exists = root_dir.open_file_in_dir(LOG_FILE, Mode::ReadOnly).is_ok();
                    if !file_exists {
                        match root_dir.open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend) {
                            Ok(file) => {
                                let _ = file.write(b"Uptime,TemperatureC\n");
                                let _ = file.flush();
                                info!("Created {} with header", LOG_FILE);
                            }
                            Err(_) => info!("Failed to create log file"),
                        }
                    }
                }
                Err(_) => info!("Failed to open root dir"),
            },
            Err(_) => info!("Failed to open volume"),
        }
    }

    // LCD (ST7789) ----------
    let mut buffer = [0u8; 512];
    let di = SpiInterface::new(lcd_spi, dc, &mut buffer);
    let mut delay = Delay;

    let mut display = Builder::new(ST7789, di)
        .display_size(240, 320)
        .orientation(Orientation::new().rotate(Rotation::Deg270))
        .invert_colors(ColorInversion::Normal)
        .init(&mut delay)
        .expect("display init failed");

    display.clear(Rgb565::BLACK).ok();

    let text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let clear_style = PrimitiveStyle::with_fill(Rgb565::BLACK);

    info!("Display initialized");

    let start = Instant::now();

    loop {
        match bme280.read_temperature().await {
            Ok(Some(temp)) => {
                let temp_int = temp as i32;
                let temp_frac = ((temp * 10.0) as i32 % 100).abs();

                info!("temp: {}.{} C", temp_int, temp_frac);

                // Обновляем показания на экране.
                Rectangle::new(Point::new(10, 10), Size::new(220, 30))
                    .into_styled(clear_style)
                    .draw(&mut display)
                    .ok();

                let mut line: String<32> = String::new();
                let _ = core::fmt::write(
                    &mut line,
                    format_args!("Temp: {}.{} C", temp_int, temp_frac),
                );
                Text::new(&line, Point::new(10, 30), text_style)
                    .draw(&mut display)
                    .ok();

                // adding CSV-log on SD
                if card_ok {
                    let elapsed_secs = (Instant::now() - start).as_secs();
                    let mut time_str: String<16> = String::new();
                    format_uptime(&mut time_str, elapsed_secs);

                    let mut csv_line: String<48> = String::new();
                    let _ = core::fmt::write(
                        &mut csv_line,
                        format_args!("{},{}.{}\n", time_str, temp_int, temp_frac),
                    );

                    match volume_mgr.open_volume(VolumeIdx(0)) {
                        Ok(volume0) => match volume0.open_root_dir() {
                            Ok(root_dir) => match root_dir
                                .open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend)
                            {
                                Ok(file) => {
                                    let _ = file.write(csv_line.as_bytes());
                                    let _ = file.flush();
                                }
                                Err(_) => info!("Failed to open log file for write"),
                            },
                            Err(_) => info!("Failed to open root dir"),
                        },
                        Err(_) => info!("Failed to open volume"),
                    }
                }
            }
            Ok(None) => info!("Measurement enabled"),
            Err(_e) => info!("Error reading"),
        }

        Timer::after_secs(1).await;
    }
}
