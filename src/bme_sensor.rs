use bme280_rs::{AsyncBme280, Configuration, Oversampling, SensorMode};
use defmt::info;

pub async fn init<I2C, D>(i2c: I2C, delay: D) -> Result<AsyncBme280<I2C, D>, ()>
where
    I2C: embedded_hal_async::i2c::I2c,
    D: embedded_hal_async::delay::DelayNs,
{
    info!("Initializing BME280...");
    let mut bme280 = AsyncBme280::new(i2c, delay);

    if let Err(e) = bme280.init().await {
        info!("BME280 init error: {:?}", defmt::Debug2Format(&e));
        return Err(());
    }
    info!("BME280 initialized");

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
        Ok(_) => info!("BME280 config done"),
        Err(_) => info!("BME280 config error"),
    }

    Ok(bme280)
}
