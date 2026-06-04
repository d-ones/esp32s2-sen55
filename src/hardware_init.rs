use embassy_time::Delay;
use embedded_hal_bus::spi::{ExclusiveDevice, NoDelay};
use esp_backtrace as _;
use esp_hal::gpio::Pin;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::i2c::master::{Config as i2cConfig, I2c};
use esp_hal::peripherals::{
    GPIO3, GPIO35, GPIO36, GPIO37, GPIO4, GPIO40, GPIO41, GPIO42, GPIO45, GPIO7, I2C0, SPI2,
};
use esp_hal::spi::master::{Config, Spi};
use esp_hal::spi::Mode;
use esp_hal::time::Rate;
use esp_hal::Async;
use mipidsi::interface::SpiInterface;
use mipidsi::options::ColorOrder;
use mipidsi::Display;
use mipidsi::{models::ST7789, Builder};
use static_cell::StaticCell;

static DISPLAY_BUFFER: StaticCell<[u8; 4096]> = StaticCell::new();

pub struct PreflightHardware {
    pub i2c0: I2C0<'static>,
    pub spi2: SPI2<'static>,
    pub pin_display_pw: GPIO7<'static>,
    pub pin_backlight: GPIO45<'static>,
    pub pin_dc: GPIO40<'static>,
    pub pin_cs: GPIO42<'static>,
    pub pin_reset: GPIO41<'static>,
    pub pin_sck: GPIO36<'static>,
    pub pin_mosi: GPIO35<'static>,
    pub pin_miso: GPIO37<'static>,
    pub pin_sda: GPIO3<'static>,
    pub pin_scl: GPIO4<'static>,
}

pub type TftDisplay<'a> = Display<
    SpiInterface<
        'a,
        ExclusiveDevice<Spi<'static, Async>, Output<'static>, NoDelay>,
        Output<'static>,
    >,
    mipidsi::models::ST7789,
    Output<'static>,
>;

pub struct DisplaySystem {
    pub tft: TftDisplay<'static>,
    pub backlight: Output<'static>,
    pub power: Output<'static>,
}

pub struct AppHardware {
    pub i2c: I2c<'static, Async>,
    pub delay: Delay,
}

pub fn init_hardware(p: PreflightHardware) -> (AppHardware, DisplaySystem) {
    // In theory we should let the fan spool
    let mut delay = Delay;

    let buffer = DISPLAY_BUFFER.init([0u8; 4096]); // Static &'static mut [u8]

    let display_power = Output::new(
        p.pin_display_pw.degrade(),
        Level::High,
        OutputConfig::default(),
    );

    let tft_backlight = Output::new(
        p.pin_backlight.degrade(),
        Level::High,
        OutputConfig::default(),
    );

    let dc = Output::new(p.pin_dc.degrade(), Level::High, OutputConfig::default());
    let cs = Output::new(p.pin_cs.degrade(), Level::High, OutputConfig::default());

    let spi = Spi::new(
        p.spi2,
        Config::default()
            .with_frequency(Rate::from_mhz(40))
            .with_mode(Mode::_0),
    )
    .unwrap()
    .with_sck(p.pin_sck.degrade())
    .with_mosi(p.pin_mosi.degrade())
    .with_miso(p.pin_miso.degrade())
    .into_async();

    let spi_device = ExclusiveDevice::new_no_delay(spi, cs).unwrap();
    let si = SpiInterface::new(spi_device, dc, buffer);

    let i2c = I2c::new(
        p.i2c0,
        i2cConfig::default().with_frequency(Rate::from_khz(100)),
    )
    .unwrap()
    .with_sda(p.pin_sda.degrade())
    .with_scl(p.pin_scl.degrade())
    .into_async();

    // Comes on bright -- entire preflight worked to here
    let display = Builder::new(ST7789, si)
        .color_order(ColorOrder::Rgb)
        .invert_colors(mipidsi::options::ColorInversion::Inverted)
        .reset_pin(Output::new(
            p.pin_reset.degrade(),
            Level::High,
            OutputConfig::default(),
        ))
        .orientation(
            mipidsi::options::Orientation::default().rotate(mipidsi::options::Rotation::Deg90),
        )
        .init(&mut delay)
        .unwrap();

    (
        AppHardware {
            i2c: i2c,
            delay: Delay,
        },
        DisplaySystem {
            tft: display,
            backlight: tft_backlight,
            power: display_power,
        },
    )
}
