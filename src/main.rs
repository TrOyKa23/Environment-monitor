#![no_std]
#![no_main]

use bme280_rs::{AsyncBme280, Configuration, Oversampling, SensorMode};
use defmt::info;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::i2c::{self, Config as I2cConfig, InterruptHandler as I2cInterruptHandler};
use embassy_rp::peripherals::I2C0;
use embassy_time::{Delay, Timer};

use {defmt_rtt as _, panic_probe as _};

// Обязательный заголовок образа для RP2350 (аналог boot2 у RP2040)
#[unsafe(link_section = ".start_block")]
#[used]
static IMAGE_DEF: embassy_rp::block::ImageDef = embassy_rp::block::ImageDef::secure_exe();

bind_interrupts!(struct Irqs {
    I2C0_IRQ => I2cInterruptHandler<I2C0>;
});

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());

    // GP4 = SDA, GP5 = SCL -> I2C0
    let sda = p.PIN_4;
    let scl = p.PIN_5;

    let i2c = i2c::I2c::new_async(p.I2C0, scl, sda, Irqs, I2cConfig::default());

    let mut bme280 = AsyncBme280::new(i2c, Delay);

    bme280
        .init()
        .await
        .expect("не удалось инициализировать BME280");

    bme280
        .set_sampling_configuration(
            Configuration::default()
                .with_temperature_oversampling(Oversampling::Oversample1)
                .with_pressure_oversampling(Oversampling::Oversample1)
                .with_humidity_oversampling(Oversampling::Oversample1)
                .with_sensor_mode(SensorMode::Normal),
        )
        .await
        .expect("не удалось настроить BME280");

    info!("BME280 готов, начинаю замеры");

    loop {
        match bme280.read_temperature().await {
            Ok(Some(temp)) => info!("Температура: {} °C", temp),
            Ok(None) => info!("Измерение температуры отключено"),
            Err(_) => info!("Ошибка чтения с BME280"),
        }

        Timer::after_secs(1).await;
    }
}
