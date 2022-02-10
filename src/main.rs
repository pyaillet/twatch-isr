#[allow(unused_imports)]
use std::thread;
use std::time::Duration;

use esp_idf_hal::delay;
use esp_idf_hal::i2c;
use esp_idf_hal::peripherals::Peripherals;
use esp_idf_hal::prelude::*;

use embedded_graphics::image::{Image, ImageRaw, ImageRawLE};
use embedded_graphics::mono_font::{ascii::FONT_10X20, MonoTextStyle};
use embedded_graphics::pixelcolor::*;
use embedded_graphics::prelude::*;
use embedded_graphics::primitives::*;
use embedded_graphics::text::*;

use embedded_hal_0_2::digital::v2::OutputPin;

mod axpxx;
mod display;

fn main() {
    // Temporary. Will disappear once ESP-IDF 4.4 is released, but for now it is necessary to call this function once,
    // or else some patches to the runtime implemented by esp-idf-sys might not link properly.
    esp_idf_sys::link_patches();

    let peripherals = Peripherals::take().unwrap();

    let i2c = peripherals.i2c0;
    let sda = peripherals.pins.gpio21; //.into_output().unwrap();
    let scl = peripherals.pins.gpio22; //.into_output().unwrap();
    let config = <i2c::config::MasterConfig as Default>::default().baudrate(400.kHz().into());
    let i2c =
        i2c::Master::<i2c::I2C0, _, _>::new(i2c, i2c::MasterPins { sda, scl }, config).unwrap();

    println!("Initializing AXP202...");
    let mut axpxx = axpxx::Axpxx::new(i2c);

    axpxx.init().unwrap();
    axpxx.init_irq().unwrap();

    axpxx
        .toggle_irq(axpxx::EventsIrq::PowerKeyShortPress, true)
        .unwrap();

    axpxx.clear_irq().unwrap();

    axpxx
        .set_power_output(axpxx::Power::Exten, axpxx::State::Off, &mut delay::Ets)
        .unwrap();
    axpxx
        .set_power_output(axpxx::Power::DcDc2, axpxx::State::Off, &mut delay::Ets)
        .unwrap();
    axpxx
        .set_power_output(axpxx::Power::Ldo4, axpxx::State::Off, &mut delay::Ets)
        .unwrap();
    axpxx
        .set_power_output(axpxx::Power::Ldo2, axpxx::State::On, &mut delay::Ets)
        .unwrap();

    let mut bl = peripherals.pins.gpio12.into_output().unwrap();
    let dc = peripherals.pins.gpio27.into_output().unwrap();
    let cs = peripherals.pins.gpio5.into_output().unwrap();
    let spiclk = peripherals.pins.gpio18.into_output().unwrap();
    let spimosi = peripherals.pins.gpio19.into_output().unwrap();

    bl.set_high().unwrap();

    println!("Initializing Display...");
    let mut display = display::new(dc, peripherals.spi2, spiclk, spimosi, cs, bl).unwrap();
    axpxx.debug_power_output().unwrap();
    display.init(&mut delay::Ets).unwrap();
    axpxx.debug_power_output().unwrap();
    display
        .set_orientation(st7789::Orientation::Portrait)
        .unwrap();
    axpxx.debug_power_output().unwrap();

    // The TTGO board's screen does not start at offset 0x0, and the physical size is 135x240, instead of 240x320
    let top_left = Point::new(0, 0);
    let size = Size::new(240, 240);

    led_draw(&mut display.cropped(&Rectangle::new(top_left, size))).unwrap();

    let mut motor = peripherals.pins.gpio4.into_output().unwrap();
    println!("Hello, world!");

    loop {
        match axpxx.is_button_pressed() {
            Ok(true) => {
                println!("Button pressed");
                motor.set_high().unwrap();
                thread::sleep(Duration::from_millis(200));
                motor.set_low().unwrap();
            }
            Ok(false) => (),
            Err(e) => println!("Error: {}", e),
        }
        thread::sleep(Duration::from_millis(100));
    }
}

#[allow(dead_code)]
fn led_draw<D>(display: &mut D) -> Result<(), D::Error>
where
    D: DrawTarget<Color = embedded_graphics::pixelcolor::Rgb565> + Dimensions,
    D::Color: From<Rgb565>,
{
    display.clear(Rgb565::WHITE.into())?;

    Rectangle::new(display.bounding_box().top_left, display.bounding_box().size)
        .into_styled(
            PrimitiveStyleBuilder::new()
                .fill_color(Rgb565::BLACK.into())
                .stroke_color(Rgb565::YELLOW.into())
                .stroke_width(1)
                .build(),
        )
        .draw(display)?;

    let raw_image: ImageRawLE<Rgb565> = ImageRaw::new(include_bytes!("../assets/ferris.raw"), 86);
    let ferris = Image::new(&raw_image, Point::new(2, 2));

    ferris.draw(display)?;

    Text::new(
        "Hello Rust!",
        Point::new(10, (display.bounding_box().size.height - 10) as i32 / 2),
        MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE.into()),
    )
    .draw(display)?;

    println!("LED rendering done");

    Ok(())
}
