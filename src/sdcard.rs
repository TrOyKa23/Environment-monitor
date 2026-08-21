use core::cell::Cell;
use core::fmt::Write as _;
use defmt::info;
use embassy_time::Delay;
use embedded_sdmmc::{Mode, SdCard, TimeSource, Timestamp, VolumeIdx, VolumeManager};
use heapless::String;

use crate::daily_history::BUCKETS;
use crate::rtc;

const LOG_FILE: &str = "TEMPLOG.CSV";

const TAIL_WINDOW_BYTES: u32 = 3_500_000;

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

    /// Читает "хвост" CSV-лога прямо с SD-карты и усредняет температуру
    /// по 48 получасовым интервалам за последние 24 часа - для графика.
    ///
    /// Важно: данные НЕ копятся в RAM между вызовами - при каждом вызове
    /// читаем нужный кусок файла заново и сразу превращаем его в 48 чисел.
    /// В памяти при этом одновременно живут только счётчики сумм/количеств
    /// (48 * (f32+u32) = ~384 байта) и небольшой буфер чтения - никакой
    /// истории показаний не хранится дольше одного вызова этой функции.
    pub fn read_daily_temps(&self) -> [Option<f32>; BUCKETS] {
        let mut sums = [0f32; BUCKETS];
        let mut counts = [0u32; BUCKETS];

        let Some(now_key) = rtc::now_unix().map(|u| u / 1800) else {
            info!("Time not synced yet - can't build daily graph");
            return [None; BUCKETS];
        };

        let result: Result<(), ()> = (|| {
            let volume0 = self.volume_mgr.open_volume(VolumeIdx(0)).map_err(|_| ())?;
            let root_dir = volume0.open_root_dir().map_err(|_| ())?;
            let file = root_dir
                .open_file_in_dir(LOG_FILE, Mode::ReadOnly)
                .map_err(|_| ())?;

            let file_len = file.length();
            let start = file_len.saturating_sub(TAIL_WINDOW_BYTES);
            file.seek_from_start(start).map_err(|_| ())?;

            let mut chunk = [0u8; 2048];
            let mut line: heapless::Vec<u8, 96> = heapless::Vec::new();
            // Если начали читать не с самого начала файла - первая "строка"
            // почти наверняка обрезана посередине, её надо просто отбросить.
            let mut skip_current_line = start != 0;

            loop {
                let n = file.read(&mut chunk).map_err(|_| ())?;
                if n == 0 {
                    break;
                }
                for &b in &chunk[..n] {
                    if b == b'\n' {
                        if !skip_current_line {
                            process_line(&line, now_key, &mut sums, &mut counts);
                        }
                        skip_current_line = false;
                        line.clear();
                    } else if b != b'\r' {
                        // Если строка почему-то длиннее буфера - просто
                        // перестаём в неё дописывать, но продолжаем ждать '\n'.
                        let _ = line.push(b);
                    }
                }
            }
            Ok(())
        })();

        if result.is_err() {
            info!("Failed to read daily history from SD");
        }

        let mut out = [None; BUCKETS];
        for i in 0..BUCKETS {
            if counts[i] > 0 {
                out[i] = Some(sums[i] / counts[i] as f32);
            }
        }
        out
    }
}

/// Разбирает одну строку CSV вида "ДД-ММ-ГГГГ,ЧЧ:ММ:СС,temp,pressure" и,
/// если она попадает в последние 24 часа, добавляет температуру в нужную
/// получасовую корзину. Некорректные/обрезанные строки просто игнорируются.
fn process_line(line: &[u8], now_key: u64, sums: &mut [f32; BUCKETS], counts: &mut [u32; BUCKETS]) {
    let Ok(text) = core::str::from_utf8(line) else {
        return;
    };

    let mut fields = text.split(',');
    let (Some(date_field), Some(time_field), Some(temp_field)) =
        (fields.next(), fields.next(), fields.next())
    else {
        return;
    };

    let Some(sample_unix) = rtc::unix_from_local_str(date_field, time_field) else {
        return;
    };
    let sample_key = sample_unix / 1800;

    // Сколько получасов назад от текущего - если строка "из будущего"
    // (не должно случаться, но на всякий случай) или старше 24ч - игнор.
    let Some(offset) = now_key.checked_sub(sample_key) else {
        return;
    };
    if offset as usize >= BUCKETS {
        return;
    }

    let Ok(temp) = temp_field.trim().parse::<f32>() else {
        return;
    };

    // offset=0 - текущий (самый свежий) получас -> последний индекс массива.
    let idx = BUCKETS - 1 - offset as usize;
    sums[idx] += temp;
    counts[idx] += 1;
}
