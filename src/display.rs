use super::hardware_init::DisplaySystem;
use super::sensiron_sensor_reading::{DisplayMessage, DATA_BUS};
use core::fmt::Write;

use embassy_time::Timer;
use embedded_graphics::{
    pixelcolor::Rgb565,
    pixelcolor::RgbColor,
    prelude::*,
    primitives::{PrimitiveStyleBuilder, Rectangle},
};
use u8g2_fonts::{fonts, FontRenderer};

pub const DEFAULT_FONT: FontRenderer = FontRenderer::new::<fonts::u8g2_font_helvB24_tr>();

#[embassy_executor::task]
pub async fn render(display_sys: &'static mut DisplaySystem) {
    let mut sub = DATA_BUS.subscriber().unwrap();
    let mut last_buf: heapless::String<32> = heapless::String::new();

    // Affirm power settings
    display_sys.backlight.set_high();
    display_sys.power.set_high();

    // Clear to a ready
    display_sys.tft.clear(Rgb565::BLACK).unwrap();

    loop {
        // poll for data
        let frame = sub.next_message_pure().await;

        let mut next_buf: heapless::String<32> = heapless::String::new();
        let write_result = match frame {
            DisplayMessage::ActiveData(_) => write!(next_buf, "OK"),
            DisplayMessage::SystemError(se) => write!(next_buf, "{}", se.code),
        };

        if write_result.is_ok() && next_buf != last_buf {
            last_buf = next_buf;

            let style = PrimitiveStyleBuilder::new()
                .fill_color(Rgb565::BLACK)
                .build();

            let _ = Rectangle::new(Point::new(0, 0), Size::new(300, 300))
                .into_styled(style)
                .draw(&mut display_sys.tft);

            let _ = DEFAULT_FONT.render_aligned(
                last_buf.as_str(),
                display_sys.tft.bounding_box().center(),
                u8g2_fonts::types::VerticalPosition::Baseline,
                u8g2_fonts::types::HorizontalAlignment::Center,
                u8g2_fonts::types::FontColor::Transparent(Rgb565::CYAN),
                &mut display_sys.tft,
            );
        }

        // debounce
        Timer::after_millis(10).await;
    }
}
