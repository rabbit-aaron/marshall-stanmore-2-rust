use std::fmt;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum SpeakerError {
    #[error("volume must be within 0 and 32")]
    InvalidVolume,
    #[error("LED brightness must be between 0 and 35")]
    InvalidLedBrightness,
    #[error("device name length must be between 1 and 17 bytes after encoding")]
    InvalidDeviceName,
    #[error("EQ band values must be within 0 and 10")]
    InvalidEqValue,
    #[error("characteristic {0} not found on device")]
    MissingCharacteristic(&'static str),
    #[error("device with address {0} not found")]
    DeviceNotFound(String),
    #[error("no bluetooth adapter found")]
    NoAdapter,
    #[error(transparent)]
    Ble(#[from] btleplug::Error),
}

/// A 5-band equalizer profile: 160hz, 400hz, 1khz, 2.5khz, 6.25khz. Each 0..=10.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EqProfile {
    pub hz160: u8,
    pub hz400: u8,
    pub hz1000: u8,
    pub hz2500: u8,
    pub hz6250: u8,
}

impl EqProfile {
    pub fn new(hz160: u8, hz400: u8, hz1000: u8, hz2500: u8, hz6250: u8) -> Result<Self, SpeakerError> {
        let profile = Self { hz160, hz400, hz1000, hz2500, hz6250 };
        for v in profile.as_bytes() {
            if v > 10 {
                return Err(SpeakerError::InvalidEqValue);
            }
        }
        Ok(profile)
    }

    pub fn from_slice(data: &[u8]) -> Result<Self, SpeakerError> {
        if data.len() < 5 {
            return Err(SpeakerError::InvalidEqValue);
        }
        Self::new(data[0], data[1], data[2], data[3], data[4])
    }

    pub fn as_bytes(&self) -> [u8; 5] {
        [self.hz160, self.hz400, self.hz1000, self.hz2500, self.hz6250]
    }

    /// Set a single band by its label ("160", "400", "1000", "2500", "6250").
    pub fn set_band(&mut self, band: &str, value: u8) -> Result<(), SpeakerError> {
        if value > 10 {
            return Err(SpeakerError::InvalidEqValue);
        }
        match band {
            "160" => self.hz160 = value,
            "400" => self.hz400 = value,
            "1000" => self.hz1000 = value,
            "2500" => self.hz2500 = value,
            "6250" => self.hz6250 = value,
            _ => return Err(SpeakerError::InvalidEqValue),
        }
        Ok(())
    }
}

impl fmt::Display for EqProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let b = self.as_bytes();
        write!(f, "{} {} {} {} {}", b[0], b[1], b[2], b[3], b[4])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EqPreset {
    Flat,
    Rock,
    Metal,
    Pop,
    HipHop,
    Electronic,
    Jazz,
}

impl EqPreset {
    pub const ALL: [EqPreset; 7] = [
        EqPreset::Flat,
        EqPreset::Rock,
        EqPreset::Metal,
        EqPreset::Pop,
        EqPreset::HipHop,
        EqPreset::Electronic,
        EqPreset::Jazz,
    ];

    pub fn profile(self) -> EqProfile {
        let (a, b, c, d, e) = match self {
            EqPreset::Flat => (5, 5, 5, 5, 5),
            EqPreset::Rock => (8, 6, 3, 5, 7),
            EqPreset::Metal => (8, 3, 5, 7, 8),
            EqPreset::Pop => (6, 7, 8, 4, 5),
            EqPreset::HipHop => (8, 7, 6, 5, 5),
            EqPreset::Electronic => (7, 4, 4, 7, 6),
            EqPreset::Jazz => (4, 7, 5, 4, 5),
        };
        EqProfile { hz160: a, hz400: b, hz1000: c, hz2500: d, hz6250: e }
    }

    pub fn from_profile(profile: EqProfile) -> Option<EqPreset> {
        EqPreset::ALL.into_iter().find(|p| p.profile() == profile)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            EqPreset::Flat => "flat",
            EqPreset::Rock => "rock",
            EqPreset::Metal => "metal",
            EqPreset::Pop => "pop",
            EqPreset::HipHop => "hiphop",
            EqPreset::Electronic => "electronic",
            EqPreset::Jazz => "jazz",
        }
    }

    pub fn parse(s: &str) -> Option<EqPreset> {
        match s.to_ascii_lowercase().as_str() {
            "flat" => Some(EqPreset::Flat),
            "rock" => Some(EqPreset::Rock),
            "metal" => Some(EqPreset::Metal),
            "pop" => Some(EqPreset::Pop),
            "hiphop" => Some(EqPreset::HipHop),
            "electronic" => Some(EqPreset::Electronic),
            "jazz" => Some(EqPreset::Jazz),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioSource {
    Bluetooth,
    Aux,
    Rca,
}

impl AudioSource {
    pub fn from_int(v: u8) -> Option<AudioSource> {
        match v {
            0x03 => Some(AudioSource::Bluetooth),
            0x01 => Some(AudioSource::Aux),
            0x04 => Some(AudioSource::Rca),
            _ => None,
        }
    }

    pub fn command_byte(self) -> u8 {
        match self {
            AudioSource::Bluetooth => 0x0C,
            AudioSource::Aux => 0x0D,
            AudioSource::Rca => 0x0E,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            AudioSource::Bluetooth => "bluetooth",
            AudioSource::Aux => "aux",
            AudioSource::Rca => "rca",
        }
    }

    pub fn parse(s: &str) -> Option<AudioSource> {
        match s.to_ascii_lowercase().as_str() {
            "bluetooth" => Some(AudioSource::Bluetooth),
            "aux" => Some(AudioSource::Aux),
            "rca" => Some(AudioSource::Rca),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlayStatus {
    Playing,
    Paused,
    Stopped,
}

impl PlayStatus {
    pub fn from_int(v: u8) -> Option<PlayStatus> {
        match v {
            0x00 => Some(PlayStatus::Playing),
            0x01 => Some(PlayStatus::Paused),
            0x02 => Some(PlayStatus::Stopped),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            PlayStatus::Playing => "playing",
            PlayStatus::Paused => "paused",
            PlayStatus::Stopped => "stopped",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Status {
    pub audio_source: AudioSource,
    pub play_status: PlayStatus,
    pub interaction_sound_enabled: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MediaInfo {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
}

/// Control-characteristic command bytes.
pub mod cmd {
    pub const PAUSE: u8 = 0x00;
    pub const PLAY: u8 = 0x01;
    pub const PREVIOUS: u8 = 0x02;
    pub const NEXT: u8 = 0x03;
    pub const DISABLE_INTERACTION_SOUND: u8 = 0x10;
    pub const ENABLE_INTERACTION_SOUND: u8 = 0x11;
}
