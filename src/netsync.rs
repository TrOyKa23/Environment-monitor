use cyw43::JoinOptions;
use cyw43_pio::{DEFAULT_CLOCK_DIVIDER, PioSpi};
use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::{Config as NetConfig, IpAddress, IpEndpoint, Stack, StackResources};
use embassy_rp::Peri;
use embassy_rp::bind_interrupts;
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{DMA_CH0, PIN_23, PIN_24, PIN_25, PIN_29, PIO0};
use embassy_rp::pio::{InterruptHandler as PioInterruptHandler, Pio};
use embassy_time::{Duration, Instant, Timer, with_timeout};
use static_cell::StaticCell;

use crate::rtc;

bind_interrupts!(struct Irqs {
    PIO0_IRQ_0 => PioInterruptHandler<PIO0>;
    DMA_IRQ_0 => embassy_rp::dma::InterruptHandler<DMA_CH0>;
});

const FW_LEN: usize = include_bytes!("43439A0.bin").len();
const CLM_LEN: usize = include_bytes!("43439A0_clm.bin").len();
const NVRAM_LEN: usize = include_bytes!("nvram_rp2040.bin").len();

static FIRMWARE: cyw43::Aligned<cyw43::A4, [u8; FW_LEN]> =
    cyw43::Aligned(*include_bytes!("43439A0.bin"));
static CLM: cyw43::Aligned<cyw43::A4, [u8; CLM_LEN]> =
    cyw43::Aligned(*include_bytes!("43439A0_clm.bin"));
static NVRAM: cyw43::Aligned<cyw43::A4, [u8; NVRAM_LEN]> =
    cyw43::Aligned(*include_bytes!("nvram_rp2040.bin"));

// unix 1900year(sec) - ntp 1970year(sec) = offset
const NTP_UNIX_OFFSET: u64 = 2_208_988_800;

const NTP_SERVER: (u8, u8, u8, u8) = (162, 159, 200, 1);

//wifi pins for cynw43439
pub struct WifiPins {
    pub pwr: Peri<'static, PIN_23>,
    pub cs: Peri<'static, PIN_25>,
    pub pio: Peri<'static, PIO0>,
    pub dio: Peri<'static, PIN_24>,
    pub clk: Peri<'static, PIN_29>,
    pub dma: Peri<'static, DMA_CH0>,
}

type Cyw43Spi = PioSpi<'static, PIO0, 0>;

#[embassy_executor::task]
async fn cyw43_task(runner: cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, Cyw43Spi>>) -> ! {
    runner.run().await
}

#[embassy_executor::task]
async fn net_task(mut runner: embassy_net::Runner<'static, cyw43::NetDriver<'static>>) -> ! {
    runner.run().await
}

//wifi + ntp sync in background. returnes stack for future (probable) reconnection
pub async fn start(
    spawner: Spawner,
    pins: WifiPins,
    ssid: &'static str,
    password: &'static str,
) -> Stack<'static> {
    let pwr = Output::new(pins.pwr, Level::Low);
    let cs = Output::new(pins.cs, Level::High);
    let mut pio = Pio::new(pins.pio, Irqs);

    let dma_channel = embassy_rp::dma::Channel::new(pins.dma, Irqs);

    let spi: Cyw43Spi = PioSpi::new(
        &mut pio.common,
        pio.sm0,
        DEFAULT_CLOCK_DIVIDER,
        pio.irq0,
        cs,
        pins.dio,
        pins.clk,
        dma_channel,
    );

    static STATE: StaticCell<cyw43::State> = StaticCell::new();
    let state = STATE.init(cyw43::State::new());
    let (net_device, mut control, runner): (
        cyw43::NetDriver<'static>,
        cyw43::Control<'static>,
        cyw43::Runner<'static, cyw43::SpiBus<Output<'static>, Cyw43Spi>>,
    ) = cyw43::new(state, pwr, spi, &FIRMWARE, &NVRAM).await;
    spawner.spawn(cyw43_task(runner).unwrap());

    control.init(&*CLM).await;
    control
        .set_power_management(cyw43::PowerManagementMode::PowerSave)
        .await;

    let net_config = NetConfig::dhcpv4(Default::default());
    let seed = Instant::now().as_ticks();

    static RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
    let (stack, runner) = embassy_net::new(
        net_device,
        net_config,
        RESOURCES.init(StackResources::new()),
        seed,
    );
    spawner.spawn(net_task(runner).unwrap());

    crate::ui::set_wifi_connecting();
    join_wifi(&mut control, ssid, password).await;

    info!("Wi-Fi: waiting for link...");
    stack.wait_link_up().await;
    info!("Wi-Fi: waiting for DHCP...");
    stack.wait_config_up().await;
    info!("Wi-Fi: connected, IP configured");
    // if NTP fails (is not connected) the WIFI is still working, so we can show "connected" already, not waiting for NTP result
    crate::ui::set_wifi_connected();

    match sync_ntp(&stack).await {
        Some(unix_secs) => {
            rtc::set_synced_time(unix_secs);
            info!("NTP: time synced ({} unix secs)", unix_secs);
        }
        None => warn!("NTP: sync failed, will keep using uptime-based timestamps"),
    }

    stack
}

async fn join_wifi(control: &mut cyw43::Control<'static>, ssid: &str, password: &str) {
    loop {
        match control
            .join(ssid, JoinOptions::new(password.as_bytes()))
            .await
        {
            Ok(()) => break,
            Err(e) => {
                warn!("Wi-Fi join failed: {:?}", defmt::Debug2Format(&e));
                Timer::after_secs(2).await;
            }
        }
    }
}

async fn sync_ntp(stack: &Stack<'static>) -> Option<u64> {
    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buf = [0u8; 512];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buf = [0u8; 512];

    let mut socket = UdpSocket::new(*stack, &mut rx_meta, &mut rx_buf, &mut tx_meta, &mut tx_buf);
    socket.bind(0).ok()?;

    let (a, b, c, d) = NTP_SERVER;
    let server = IpEndpoint::new(IpAddress::v4(a, b, c, d), 123);

    let mut request = [0u8; 48];
    request[0] = 0x1B;

    socket.send_to(&request, server).await.ok()?;

    let mut reply = [0u8; 48];
    let (n, _) = with_timeout(Duration::from_secs(5), socket.recv_from(&mut reply))
        .await
        .ok()?
        .ok()?;
    if n < 48 {
        return None;
    }

    // From 40 to 43 bytes of NTP reply take the date/time
    let secs = u32::from_be_bytes([reply[40], reply[41], reply[42], reply[43]]) as u64;
    Some(secs.saturating_sub(NTP_UNIX_OFFSET))
}
