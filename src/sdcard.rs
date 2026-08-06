use core::fmt::Write as _;
use defmt::info;
use embassy_time::Delay;
use embedded_sdmmc::{Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use heapless::String;

const LOG_FILE: &str = "TEMPLOG.CSV";

// TIP 1
#[derive(Default)]
pub struct DummyTimesource;

impl TimeSource for DummyTimesource {
    fn get_timestamp(&self) -> Timestamp {
        Timestamp {
            year_since_1970: 0,
            zero_indexed_month: 0,
            zero_indexed_day: 0,
            hours: 0,
            minutes: 0,
            seconds: 0,
        }
    }
}

// log write
pub struct TempLogger<SPI>
where
    SPI: embedded_hal::spi::SpiDevice,
{
    volume_mgr: VolumeManager<SdCard<SPI, Delay>, DummyTimesource>,
}

impl<SPI> TempLogger<SPI>
where
    SPI: embedded_hal::spi::SpiDevice,
{
    // sd card read check
    pub fn new(spi_device: SPI) -> Option<Self> {
        let sdcard = SdCard::new(spi_device, Delay);
        match sdcard.num_bytes() {
            Ok(size) => info!("SD card size: {} bytes", size),
            Err(_) => {
                info!("SD card init failed - logging to SD disabled");
                return None;
            }
        }

        let volume_mgr = VolumeManager::new(sdcard, DummyTimesource);

        match volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(volume0) => match volume0.open_root_dir() {
                Ok(root_dir) => {
                    // open file for log create. if exists, do not create new file
                    let file_exists = root_dir.open_file_in_dir(LOG_FILE, Mode::ReadOnly).is_ok();
                    if !file_exists {
                        match root_dir.open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend) {
                            Ok(file) => {
                                let _ = file
                                    .write(b"Uptime,TemperatureC,HumidityPercent,PressureHPa\n");
                                let _ = file.flush();
                                info!("Created {} with header", LOG_FILE);
                            }
                            Err(_) => info!("Failed to create log file"),
                        }
                    }
                }
                Err(_) => info!("Failed to open root dir"),
            },
            Err(_) => info!("Failed to open volume"),
        }

        Some(Self { volume_mgr })
    }

    // log sample to file (+ 1 row of data)
    pub fn log_sample(&self, uptime_secs: u64, temp_c: f32, humidity_pct: f32, pressure_hpa: f32) {
        let h = uptime_secs / 3600;
        let m = (uptime_secs % 3600) / 60;
        let s = uptime_secs % 60;

        let mut csv_line: String<64> = String::new();
        let _ = write!(
            csv_line,
            "{:02}:{:02}:{:02},{:.1},{:.1},{:.1}\n",
            h, m, s, temp_c, humidity_pct, pressure_hpa
        );

        match self.volume_mgr.open_volume(VolumeIdx(0)) {
            Ok(volume0) => match volume0.open_root_dir() {
                Ok(root_dir) => {
                    match root_dir.open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend) {
                        Ok(file) => {
                            let _ = file.write(csv_line.as_bytes());
                            let _ = file.flush();
                        }
                        Err(_) => info!("Failed to open log file for write"),
                    }
                }
                Err(_) => info!("Failed to open root dir"),
            },
            Err(_) => info!("Failed to open volume"),
        }
    }
}
