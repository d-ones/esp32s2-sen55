use crate::sensiron_sensor_reading::Sen55Packed;

use super::secrets::{WIFI_PASS, WIFI_SSID};
use super::sensiron_sensor_reading::{DisplayMessage, DATA_BUS};
use core::fmt::Write;
use embassy_net::Stack;
use embassy_time::Timer;
use esp_backtrace as _;
use esp_radio::wifi::{
    AuthMethod, ClientConfig, ModeConfig, WifiController, WifiDevice, WifiStaState,
};

#[embassy_executor::task]
pub async fn connection_task(mut controller: WifiController<'static>) {
    // wait for the S2 power rails to settle
    Timer::after_millis(500).await;

    loop {
        if esp_radio::wifi::sta_state() != WifiStaState::Connected {
            let config = ModeConfig::Client(
                ClientConfig::default()
                    .with_auth_method(AuthMethod::Wpa2Personal)
                    .with_ssid(WIFI_SSID.try_into().unwrap())
                    .with_password(WIFI_PASS.try_into().unwrap()),
            );
            if controller.set_config(&config).is_ok() {
                if !controller.is_started().unwrap_or(false) {
                    controller.start_async().await.ok();
                };
                match controller.connect_async().await {
                    Ok(_) => esp_println::println!("WiFi: Connected!"),
                    Err(e) => esp_println::println!("WiFi: Connect failed {:?}", e),
                }
            }
        }

        Timer::after_secs(10).await;
    }
}

#[embassy_executor::task]
pub async fn net_task(mut runner: embassy_net::Runner<'static, WifiDevice<'static>>) {
    runner.run().await
}

#[embassy_executor::task]
pub async fn send_data(stack: &'static Stack<'static>, remote_ip: [u8; 4], port: u16) {
    let mut rx_payload = [0u8; 64];
    let mut tx_payload = [0u8; 512];
    let mut socket = embassy_net::tcp::TcpSocket::new(*stack, &mut rx_payload, &mut tx_payload);

    let remote_endpoint = embassy_net::IpEndpoint::new(
        embassy_net::IpAddress::v4(remote_ip[0], remote_ip[1], remote_ip[2], remote_ip[3]),
        port,
    );

    let mut sub = DATA_BUS.subscriber().unwrap();

    loop {
        let frame = sub.next_message_pure().await;
        if stack.is_config_up() {
            if socket.connect(remote_endpoint).await.is_ok() {
                let _ = match frame {
                    DisplayMessage::ActiveData(ad) => {
                        let ptr = &Sen55Packed::new(&ad) as *const Sen55Packed as *const u8;
                        let slice = unsafe { core::slice::from_raw_parts(ptr, 28) };
                        use embedded_io_async::Write as _;
                        let _ = socket.write_all(slice).await;
                    }
                    DisplayMessage::SystemError(se) => {
                        let mut s = heapless::String::<28>::new();
                        let _ = write!(s, "{}\n", se.long_message);
                        use embedded_io_async::Write as _;
                        let _ = socket.write_all(s.as_bytes()).await;
                    }
                };
                let _ = socket.flush().await;

                socket.close();
            } else {
                socket.abort();

                let _ = socket.flush().await;
            }
        }
        // debounce
        Timer::after_millis(50).await;
    }
}
