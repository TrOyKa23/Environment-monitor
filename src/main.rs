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

    info!("Initializing BME280...");

    let mut bme280 = AsyncBme280::new(i2c, Delay);

    match bme280.init().await {
        Ok(_) => {
            info!("✓ BME280 inicialized");
        }
        Err(e) => {
            info!("❌ Inicialisation error: {:?}", defmt::Debug2Format(&e));
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
        Ok(_) => info!("✓ Config done"),
        Err(_) => info!("❌ Error config"),
    }

    loop {
        match bme280.read_temperature().await {
            Ok(Some(temp)) => {
                let temp_int = temp as i32;
                let temp_frac = ((temp * 10.0) as i32 % 100).abs();
                info!("temp: {}.{} C", temp_int, temp_frac);
            }
            Ok(None) => info!("Measurement enabled"),
            Err(_e) => info!("Error reading"),
        }

        Timer::after_secs(1).await;
    }
}
