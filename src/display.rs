use display_interface_spi::SPIInterfaceNoCS;

use esp_idf_hal::gpio::Output;
use esp_idf_hal::gpio::{self, Unknown};
use esp_idf_hal::prelude::*;
use esp_idf_hal::spi::{self, Master};

use esp_idf_sys::EspError;

use st7789::ST7789;

pub fn new(
    dc: gpio::Gpio27<gpio::Output>,
    spi: spi::SPI2,
    sclk: gpio::Gpio18<gpio::Output>,
    sdo: gpio::Gpio19<gpio::Output>,
    cs: gpio::Gpio5<gpio::Output>,
    bl: gpio::Gpio12<gpio::Output>,
) -> Result<
    ST7789<
        SPIInterfaceNoCS<
            Master<
                esp_idf_hal::spi::SPI2,
                gpio::Gpio18<Output>,
                gpio::Gpio19<Output>,
                gpio::Gpio21<Unknown>,
                gpio::Gpio5<Output>,
            >,
            gpio::Gpio27<Output>,
        >,
        gpio::Gpio12<Output>,
    >,
    EspError,
> {
    let config = <spi::config::Config as Default>::default()
        .baudrate(26.MHz().into())
        // .bit_order(embedded_hal::spi::BitOrder::MSBFirst)
        .data_mode(embedded_hal::spi::MODE_0);

    let spi = spi::Master::<spi::SPI2, _, _, _, _>::new(
        spi,
        spi::Pins {
            sclk,
            sdo,
            sdi: Option::<gpio::Gpio21<gpio::Unknown>>::None,
            cs: Some(cs),
        },
        config,
    )?;
    let di = SPIInterfaceNoCS::new(spi, dc.into_output()?);

    let display = st7789::ST7789::new(
        di,
        None,
        Some(bl),
        // SP7789V is designed to drive 240x320 screens, even though the TTGO physical screen is smaller
        240,
        240,
    );
    Ok(display)
}
