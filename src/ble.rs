use std::str::FromStr;
use std::time::Duration;

use btleplug::api::{
    Central, CharPropFlags, Characteristic, Manager as _, Peripheral as _, ScanFilter, WriteType,
};
use btleplug::platform::{Manager, Peripheral};
use futures::stream::Stream;
use std::pin::Pin;
use tracing::{debug, info};
use uuid::Uuid;

use crate::types::{
    cmd, AudioSource, EqPreset, EqProfile, MediaInfo, SpeakerError, Status,
};

// The original UUIDs were written as 8 groups of 4 hex chars. Regrouped here into
// the canonical 8-4-4-4-12 layout.
const VOLUME_UUID: Uuid = Uuid::from_u128(0x44fa50b2_d0a3_472e_a939_d80cf17638bb);
const CONTROL_UUID: Uuid = Uuid::from_u128(0x4446cf5f_12f2_4c1e_afe1_b15797535ba8);
const LED_BRIGHTNESS_UUID: Uuid = Uuid::from_u128(0x35e3b090_1d43_35ae_af35_d254b153fc36);
const DEVICE_NAME_UUID: Uuid = Uuid::from_u128(0x3ba91c2e_8b08_4c27_9d4e_4936a793fcfb);
const EQ_UUID: Uuid = Uuid::from_u128(0x31fbb033_1013_bd3e_a249_d856f156a319);
const PAIRING_UUID: Uuid = Uuid::from_u128(0x4a75c20f_13bd_44a1_b39d_a70f86f607a2);
const MEDIA_INFO_UUID: Uuid = Uuid::from_u128(0x95c09f26_95a4_4597_a798_b8e408f5ca66);

const MEDIA_INFO_END: [u8; 8] = [0x00, 0x00, 0x00, 0xFF, 0x00, 0x00, 0x00, 0x00];

/// A decoded value pushed by the speaker over a notify characteristic.
#[derive(Debug, Clone)]
pub enum Notification {
    Volume(u8),
    Status(Status),
    Equalizer(EqProfile),
    Media(MediaInfo),
}

#[derive(Clone)]
pub struct Stanmore {
    peripheral: Peripheral,
    volume: Characteristic,
    control: Characteristic,
    led_brightness: Characteristic,
    device_name: Characteristic,
    eq: Characteristic,
    pairing: Characteristic,
    media_info: Characteristic,
}

fn find_char(
    peripheral: &Peripheral,
    uuid: Uuid,
    name: &'static str,
) -> Result<Characteristic, SpeakerError> {
    peripheral
        .characteristics()
        .into_iter()
        .find(|c| c.uuid == uuid)
        .ok_or(SpeakerError::MissingCharacteristic(name))
}

impl Stanmore {
    /// Scan for and connect to the speaker at `address`, then discover services and
    /// subscribe to notify characteristics.
    pub async fn connect(address: &str) -> Result<Self, SpeakerError> {
        let manager = Manager::new().await?;
        let adapter = manager
            .adapters()
            .await?
            .into_iter()
            .next()
            .ok_or(SpeakerError::NoAdapter)?;

        info!("Scanning for {address} ...");
        adapter.start_scan(ScanFilter::default()).await?;

        let target = btleplug::api::BDAddr::from_str(address)
            .map_err(|_| SpeakerError::DeviceNotFound(address.to_string()))?;

        let peripheral = {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
            loop {
                let found = adapter
                    .peripherals()
                    .await?
                    .into_iter()
                    .find(|p| p.address() == target);
                if let Some(p) = found {
                    break p;
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(SpeakerError::DeviceNotFound(address.to_string()));
                }
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
        };

        let _ = adapter.stop_scan().await;

        info!("Connecting ...");
        peripheral.connect().await?;
        info!("Connected, discovering services ...");
        peripheral.discover_services().await?;

        let speaker = Self {
            volume: find_char(&peripheral, VOLUME_UUID, "volume")?,
            control: find_char(&peripheral, CONTROL_UUID, "control")?,
            led_brightness: find_char(&peripheral, LED_BRIGHTNESS_UUID, "led_brightness")?,
            device_name: find_char(&peripheral, DEVICE_NAME_UUID, "device_name")?,
            eq: find_char(&peripheral, EQ_UUID, "eq")?,
            pairing: find_char(&peripheral, PAIRING_UUID, "pairing")?,
            media_info: find_char(&peripheral, MEDIA_INFO_UUID, "media_info")?,
            peripheral,
        };

        for c in [
            &speaker.control,
            &speaker.volume,
            &speaker.media_info,
            &speaker.eq,
        ] {
            if c.properties.contains(CharPropFlags::NOTIFY) {
                speaker.peripheral.subscribe(c).await?;
                info!("Subscribed to {}", c.uuid);
            }
        }

        Ok(speaker)
    }

    pub async fn notifications(
        &self,
    ) -> Result<Pin<Box<dyn Stream<Item = btleplug::api::ValueNotification> + Send>>, SpeakerError>
    {
        Ok(self.peripheral.notifications().await?)
    }

    /// Decode a raw notification into a typed [`Notification`], buffering partial
    /// media-info packets in `media_buf` until a complete message arrives.
    pub fn decode_notification(
        uuid: Uuid,
        data: &[u8],
        media_buf: &mut Vec<u8>,
    ) -> Option<Notification> {
        match uuid {
            VOLUME_UUID => data.first().map(|v| Notification::Volume(*v)),
            CONTROL_UUID => decode_status(data).map(Notification::Status),
            EQ_UUID => EqProfile::from_slice(data).ok().map(Notification::Equalizer),
            MEDIA_INFO_UUID => {
                media_buf.extend_from_slice(data);
                if data.len() >= MEDIA_INFO_END.len()
                    && data[data.len() - MEDIA_INFO_END.len()..] == MEDIA_INFO_END
                {
                    let info = decode_media_info(media_buf);
                    media_buf.clear();
                    Some(Notification::Media(info))
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    async fn write(&self, c: &Characteristic, data: &[u8]) -> Result<(), SpeakerError> {
        self.peripheral
            .write(c, data, WriteType::WithResponse)
            .await?;
        Ok(())
    }

    async fn send_command(&self, byte: u8) -> Result<(), SpeakerError> {
        self.write(&self.control, &[byte]).await
    }

    pub async fn get_status(&self) -> Result<Status, SpeakerError> {
        let data = self.peripheral.read(&self.control).await?;
        decode_status(&data).ok_or(SpeakerError::Ble(btleplug::Error::Other(
            "could not decode status".into(),
        )))
    }

    pub async fn set_volume(&self, volume: u8) -> Result<(), SpeakerError> {
        if volume > 32 {
            return Err(SpeakerError::InvalidVolume);
        }
        self.write(&self.volume, &[volume]).await
    }

    pub async fn get_volume(&self) -> Result<u8, SpeakerError> {
        let data = self.peripheral.read(&self.volume).await?;
        Ok(*data.first().unwrap_or(&0))
    }

    pub async fn set_source(&self, source: AudioSource) -> Result<(), SpeakerError> {
        self.send_command(source.command_byte()).await
    }

    pub async fn play(&self) -> Result<(), SpeakerError> {
        self.send_command(cmd::PLAY).await
    }

    pub async fn pause(&self) -> Result<(), SpeakerError> {
        self.send_command(cmd::PAUSE).await
    }

    pub async fn next(&self) -> Result<(), SpeakerError> {
        self.send_command(cmd::NEXT).await
    }

    pub async fn previous(&self) -> Result<(), SpeakerError> {
        self.send_command(cmd::PREVIOUS).await
    }

    pub async fn set_interaction_sound(&self, enabled: bool) -> Result<(), SpeakerError> {
        let byte = if enabled {
            cmd::ENABLE_INTERACTION_SOUND
        } else {
            cmd::DISABLE_INTERACTION_SOUND
        };
        self.send_command(byte).await
    }

    pub async fn set_led_brightness(&self, brightness: u8) -> Result<(), SpeakerError> {
        if brightness > 35 {
            return Err(SpeakerError::InvalidLedBrightness);
        }
        self.write(&self.led_brightness, &[brightness + 35]).await
    }

    pub async fn get_led_brightness(&self) -> Result<u8, SpeakerError> {
        let data = self.peripheral.read(&self.led_brightness).await?;
        Ok(data.first().copied().unwrap_or(35).saturating_sub(35))
    }

    pub async fn set_device_name(&self, name: &str) -> Result<(), SpeakerError> {
        let encoded = name.as_bytes();
        if encoded.is_empty() || encoded.len() > 17 {
            return Err(SpeakerError::InvalidDeviceName);
        }
        let mut data = Vec::with_capacity(encoded.len() + 2);
        data.push(0x01);
        data.push(encoded.len() as u8);
        data.extend_from_slice(encoded);
        self.write(&self.device_name, &data).await
    }

    pub async fn get_device_name(&self) -> Result<String, SpeakerError> {
        let data = self.peripheral.read(&self.device_name).await?;
        let start = data.len().min(2);
        Ok(String::from_utf8_lossy(&data[start..]).into_owned())
    }

    pub async fn set_eq_profile(&self, profile: EqProfile) -> Result<(), SpeakerError> {
        self.write(&self.eq, &profile.as_bytes()).await
    }

    pub async fn get_eq_profile(&self) -> Result<EqProfile, SpeakerError> {
        let data = self.peripheral.read(&self.eq).await?;
        EqProfile::from_slice(&data)
    }

    pub async fn set_eq_preset(&self, preset: EqPreset) -> Result<(), SpeakerError> {
        self.set_eq_profile(preset.profile()).await
    }

    pub async fn enter_pairing_mode(&self) -> Result<(), SpeakerError> {
        self.write(&self.pairing, &[0]).await
    }
}

fn decode_status(data: &[u8]) -> Option<Status> {
    if data.len() < 4 {
        return None;
    }
    Some(Status {
        audio_source: AudioSource::from_int(data[0])?,
        play_status: crate::types::PlayStatus::from_int(data[1])?,
        interaction_sound_enabled: data[3] == 1,
    })
}

/// Parse the accumulated media-info buffer. Each field is preceded by a 7-byte
/// marker `00 00 00 <idx> 00 6a 00` (idx: 1=title, 2=artist, 3=album), then a
/// length byte and the UTF-8 payload.
fn decode_media_info(data: &[u8]) -> MediaInfo {
    let decode_field = |idx: u8| -> Option<String> {
        let marker = [0x00, 0x00, 0x00, idx, 0x00, 0x6A, 0x00];
        let pos = data.windows(marker.len()).position(|w| w == marker)?;
        let len_pos = pos + marker.len();
        let start = len_pos + 1;
        let len = *data.get(len_pos)? as usize;
        let end = start + len;
        if end > data.len() {
            return None;
        }
        Some(String::from_utf8_lossy(&data[start..end]).into_owned())
    };

    let info = MediaInfo {
        title: decode_field(0x01),
        artist: decode_field(0x02),
        album: decode_field(0x03),
    };
    debug!("Decoded media info: {info:?}");
    info
}
