#![no_std]
#![no_main]
use airqual::display::render;
use airqual::hardware_init::{init_hardware, AppHardware, DisplaySystem, PreflightHardware};
use airqual::net::{connection_task, send_data};
use airqual::secrets::{DESTINATION_IP, PORT};
use airqual::sensiron_sensor_reading::{DisplayMessage, ErrorMeta, Sen55Frame, DATA_BUS};
use embassy_executor::Spawner;
use embassy_net::{new as embassy_new, DhcpConfig, Stack, StackResources};
use embassy_time::{Duration, Ticker};

use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::rng::Rng;
use esp_hal::rtc_cntl::sleep;
use esp_hal::timer::timg::TimerGroup;
use esp_radio::{init, wifi::WifiDevice};
use static_cell::StaticCell;

extern crate alloc;

esp_bootloader_esp_idf::esp_app_desc!();

// Managing peripherals across embassy timer loop + preflight init
static HW: StaticCell<AppHardware> = StaticCell::new();
static STACK_CELL: StaticCell<Stack<'static>> = StaticCell::new();
static RADIO_CELL: StaticCell<esp_radio::Controller<'static>> = StaticCell::new();
static STACK_RESOURCES: StaticCell<StackResources<3>> = StaticCell::new();
static DISPLAY: StaticCell<DisplaySystem> = StaticCell::new();

#[esp_rtos::main]
async fn main(spawner: Spawner) -> ! {
    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    let timg0 = TimerGroup::new(peripherals.TIMG0);
    let timer0 = timg0.timer0;
    esp_rtos::start(timer0);

    let p = PreflightHardware {
        i2c0: peripherals.I2C0,
        spi2: peripherals.SPI2,
        pin_display_pw: peripherals.GPIO7,
        pin_backlight: peripherals.GPIO45,
        pin_dc: peripherals.GPIO40,
        pin_cs: peripherals.GPIO42,
        pin_reset: peripherals.GPIO41,
        pin_sck: peripherals.GPIO36,
        pin_mosi: peripherals.GPIO35,
        pin_miso: peripherals.GPIO37,
        pin_sda: peripherals.GPIO3,
        pin_scl: peripherals.GPIO4,
    };
    let (hw_instance, disp_instance) = init_hardware(p);

    let hw = HW.init(hw_instance);

    let disp = DISPLAY.init(disp_instance);

    // Slate of appeasements to the WiFi
    esp_alloc::heap_allocator!(size: 64 * 1024);

    esp_hal::interrupt::enable(
        esp_hal::peripherals::Interrupt::WIFI_PWR,
        esp_hal::interrupt::Priority::Priority1,
    )
    .unwrap();

    esp_hal::interrupt::enable(
        esp_hal::peripherals::Interrupt::WIFI_MAC,
        esp_hal::interrupt::Priority::Priority1,
    )
    .unwrap();

    let rng = Rng::new();

    let radio_controller = init().expect("Radio Init failed");

    let radio_init = RADIO_CELL.init(radio_controller);

    let wifi_config = esp_radio::wifi::Config::default()
        .with_static_rx_buf_num(4)
        .with_ampdu_rx_enable(false);

    let (wifi_controller, interfaces) =
        esp_radio::wifi::new(radio_init, peripherals.WIFI, wifi_config).unwrap();

    let net_seed = (rng.random() as u64) << 32 | (rng.random() as u64);

    let (stack, runner) = embassy_new(
        interfaces.sta,
        embassy_net::Config::dhcpv4(DhcpConfig::default()),
        STACK_RESOURCES.init(StackResources::<3>::new()),
        net_seed,
    );

    let stack_ref = STACK_CELL.init(stack);
    // END wifi appeasements

    // Slate of async background tasks

    spawner.spawn(render(disp)).unwrap();

    spawner
        .spawn(net_task(runner))
        .expect("Failed to spawn Wi-Fi bootstrap task");

    spawner.spawn(connection_task(wifi_controller)).unwrap();

    spawner
        .spawn(send_data(stack_ref, DESTINATION_IP, PORT))
        .unwrap();
    // END slate of background tasks

    // Sensiron code
    let publisher = DATA_BUS.publisher().unwrap();
    let sen55_addr = 0x69;

    // Take a breath
    embassy_time::Timer::after(Duration::from_secs(15)).await;

    // UDP/display debug
    publisher
        .publish(DisplayMessage::SystemError(ErrorMeta {
            code: "HI",
            long_message: "Hello from the machine",
        }))
        .await;

    let mut ticker = Ticker::every(Duration::from_secs(600));

    loop {
        // 120s to spin up the fan & get NOX / VOC sensors outputting steady readings (empirical)
        let _ = hw.i2c.write(sen55_addr, &[0x00, 0x21]);
        embassy_time::Timer::after(Duration::from_secs(120)).await;

        for attempt in 0..=5 {
            let poll_delay = Duration::from_millis(100 * (1 << attempt));
            // Poll ready status
            let mut ready = false;
            for _ in 0..10 {
                embassy_time::Timer::after(poll_delay).await;
                let mut ready_buffer = [0u8; 3];

                let _ = hw.i2c.write(sen55_addr, &[0x02, 0x02]);
                embassy_time::Timer::after(Duration::from_millis(20)).await; // Critical gap

                if let Ok(_) = hw.i2c.read(sen55_addr, &mut ready_buffer) {
                    // Logically: [MSB, LSB, CRC] -> we want bit 0 of the LSB
                    if (ready_buffer[1] & 0x01) != 0 {
                        ready = true;
                        break;
                    }
                }

                // poll sleep
                embassy_time::Timer::after(Duration::from_millis(500)).await;
            }

            if ready {
                let mut i2c_buffer = [0u8; 24];

                // Command: Read Measured Values (0x03C4)
                if let Ok(_) = hw.i2c.write(sen55_addr, &[0x03, 0xC4]) {
                    // 20ms to move data to its I2C buffer
                    embassy_time::Timer::after(Duration::from_millis(20)).await;

                    if let Ok(_) = hw.i2c.read(sen55_addr, &mut i2c_buffer) {
                        // PARSE
                        if let Some(frame) = Sen55Frame::parse(&i2c_buffer) {
                            let _ = publisher.try_publish(DisplayMessage::ActiveData(frame));
                        } else {
                            publisher
                                .publish(DisplayMessage::SystemError(ErrorMeta {
                                    code: "CRC",
                                    long_message: "CRC Failed from SEN55",
                                }))
                                .await;
                        }
                    } else {
                        // read transaction failed
                        publisher
                            .publish(DisplayMessage::SystemError(ErrorMeta {
                                code: "COM",
                                long_message: "Communication with SEN55 failed",
                            }))
                            .await;
                    }
                }
            } else {
                // never saw "Ready"
                publisher
                    .publish(DisplayMessage::SystemError(ErrorMeta {
                        code: "NRD",
                        long_message: "SEN55 not ready",
                    }))
                    .await;
            }

            embassy_time::Timer::after(Duration::from_secs(5)).await;
        }

        let _ = hw.i2c.write(sen55_addr, &[0x01, 0x04]);
        ticker.next().await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: embassy_net::Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}
