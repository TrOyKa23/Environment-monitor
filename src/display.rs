use embedded_graphics::pixelcolor::Rgb565;
use embedded_graphics::prelude::*;
use mipidsi::Builder;
use mipidsi::interface::SpiInterface;
use mipidsi::models::ST7789;
use mipidsi::options::{ColorInversion, Orientation, Rotation};

pub fn init<'a, SPI, DC, DELAY>(
    spi_device: SPI,
    dc: DC,
    buffer: &'a mut [u8],
    delay: &mut DELAY,
) -> impl DrawTarget<Color = Rgb565> + 'a
where
    SPI: embedded_hal::spi::SpiDevice + 'a,
    DC: embedded_hal::digital::OutputPin + 'a,
    DELAY: embedded_hal::delay::DelayNs,
{
    let di = SpiInterface::new(spi_device, dc, buffer);

    Builder::new(ST7789, di)
        .display_size(240, 320)
        .orientation(Orientation::new().rotate(Rotation::Deg270))
        .invert_colors(ColorInversion::Normal)
        .init(delay)
        .expect("display init failed")
}
