use core::cell::Cell;
use core::fmt::Write as _;
use defmt::info;
use embassy_time::Delay;
use embedded_sdmmc::{Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use heapless::String;

use crate::rtc;

const LOG_FILE: &str = "TEMPLOG.CSV";

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
    //no spam log if card removed
    card_ok: Cell<bool>,
}

impl<SPI> TempLogger<SPI>
where
    SPI: embedded_hal::spi::SpiDevice,
{
    // no file if no time sync
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
        Some(Self {
            volume_mgr,
            card_ok: Cell::new(true),
        })
    }

    // reinitialize card
    fn reset_card(&self) {
        self.volume_mgr.device(|dev| dev.mark_card_uninit());
    }

    // no log if no time sync
    pub fn log_sample(&self, temp_c: f32, pressure_hpa: f32) {
        let Some(dt) = rtc::now_datetime() else {
            info!("Time not synced yet - skipping log entry");
            return;
        };

        let mut csv_line: String<64> = String::new();
        let _ = write!(
            csv_line,
            "{:02}-{:02}-{:04},{:02}:{:02}:{:02},{:.1},{:.1}\n",
            dt.day, dt.month, dt.year, dt.hour, dt.minute, dt.second, temp_c, pressure_hpa
        );

        let result: Result<(), ()> = (|| {
            let volume0 = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
            let root_dir = volume0.open_root_dir().map_err(|_| ())?;

            let file_exists = root_dir.open_file_in_dir(LOG_FILE, Mode::ReadOnly).is_ok();
            if !file_exists {
                let header_file = root_dir
                    .open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend)
                    .map_err(|_| ())?;
                header_file
                    .write(b"Date,Time,TemperatureC,PressureHPa\n")
                    .map_err(|_| ())?;
                header_file.flush().map_err(|_| ())?;
                info!("Created {} with header", LOG_FILE);
            }

            let file = root_dir
                .open_file_in_dir(LOG_FILE, Mode::ReadWriteCreateOrAppend)
                .map_err(|_| ())?;
            file.write(csv_line.as_bytes()).map_err(|_| ())?;
            file.flush().map_err(|_| ())?;
            Ok(())
        })();

        match result {
            Ok(()) => {
                if !self.card_ok.replace(true) {
                    info!("SD card reconnected - resuming logging");
                }
            }
            Err(()) => {
                if self.card_ok.replace(false) {
                    info!("SD card write failed (removed?) - will retry once it's back");
                }

                self.reset_card();
            }
        }
    }
}
