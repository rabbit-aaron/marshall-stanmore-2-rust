use std::time::Duration;

use rumqttc::{AsyncClient, QoS};
use tokio::sync::Mutex;
use tracing::{error, info, warn};

use crate::ble::{Notification, Stanmore};
use crate::types::{AudioSource, EqPreset, EqProfile, MediaInfo, Status};

const REFRESH_DELAY: Duration = Duration::from_millis(500);
const EQ_BANDS: [&str; 5] = ["160", "400", "1000", "2500", "6250"];

/// Holds the connected speaker + MQTT client and translates between the two.
pub struct App {
    speaker: Stanmore,
    client: AsyncClient,
    prefix: String,
    retain: bool,
    allow_pairing: bool,
    /// Serializes BLE access so concurrent commands don't interleave on the wire.
    ble_lock: Mutex<()>,
}

impl App {
    pub fn new(
        speaker: Stanmore,
        client: AsyncClient,
        prefix: String,
        retain: bool,
        allow_pairing: bool,
    ) -> Self {
        Self { speaker, client, prefix, retain, allow_pairing, ble_lock: Mutex::new(()) }
    }

    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    pub async fn notifications(
        &self,
    ) -> Result<
        std::pin::Pin<
            Box<dyn futures::stream::Stream<Item = btleplug::api::ValueNotification> + Send>,
        >,
        crate::types::SpeakerError,
    > {
        self.speaker.notifications().await
    }

    fn topic(&self, suffix: &str) -> String {
        format!("{}/{}", self.prefix, suffix)
    }

    async fn publish(&self, topic: String, payload: impl Into<Vec<u8>>) {
        self.publish_retain(topic, payload, self.retain).await;
    }

    pub async fn publish_retain(&self, topic: String, payload: impl Into<Vec<u8>>, retain: bool) {
        let payload = payload.into();
        if let Err(e) = self
            .client
            .publish(&topic, QoS::AtLeastOnce, retain, payload)
            .await
        {
            error!("Failed to publish to {topic}: {e}");
        }
    }

    /// Read every value from the speaker and publish it, e.g. on startup so Home
    /// Assistant immediately reflects current state.
    pub async fn publish_all_state(&self) {
        let _guard = self.ble_lock.lock().await;
        self.refresh_status().await;
        self.refresh_volume().await;
        self.refresh_led_brightness().await;
        self.refresh_eq().await;
        self.refresh_device_name().await;
    }

    // --- notification (push) handling -------------------------------------

    pub async fn on_notification(&self, notification: Notification) {
        match notification {
            Notification::Volume(v) => {
                self.publish(self.topic("info/volume"), v.to_string()).await;
            }
            Notification::Status(s) => self.publish_status(s).await,
            Notification::Equalizer(eq) => self.publish_eq(eq).await,
            Notification::Media(info) => self.publish_media(info).await,
        }
    }

    async fn publish_status(&self, status: Status) {
        self.publish(self.topic("info/play_status"), status.play_status.as_str())
            .await;
        self.publish(self.topic("info/audio_source"), status.audio_source.as_str())
            .await;
        self.publish(
            self.topic("info/interaction_sound_enabled"),
            if status.interaction_sound_enabled { "1" } else { "0" },
        )
        .await;
    }

    async fn publish_eq(&self, eq: EqProfile) {
        self.publish(self.topic("info/eq_profile"), eq.to_string()).await;
        let bands = eq.as_bytes();
        for (band, value) in EQ_BANDS.iter().zip(bands.iter()) {
            self.publish(self.topic(&format!("info/eq_profile/{band}hz")), value.to_string())
                .await;
        }
        let preset = EqPreset::from_profile(eq);
        self.publish_eq_preset(preset).await;
    }

    async fn publish_eq_preset(&self, preset: Option<EqPreset>) {
        let name = preset.map(|p| p.as_str()).unwrap_or("custom");
        self.publish(self.topic("info/eq_preset"), name).await;
    }

    async fn publish_media(&self, info: MediaInfo) {
        self.publish(self.topic("info/media/title"), info.title.unwrap_or_default())
            .await;
        self.publish(self.topic("info/media/artist"), info.artist.unwrap_or_default())
            .await;
        self.publish(self.topic("info/media/album"), info.album.unwrap_or_default())
            .await;
    }

    // --- refresh helpers (read from speaker then publish) -----------------

    async fn refresh_status(&self) {
        match self.speaker.get_status().await {
            Ok(s) => self.publish_status(s).await,
            Err(e) => error!("get_status failed: {e}"),
        }
    }

    async fn refresh_volume(&self) {
        match self.speaker.get_volume().await {
            Ok(v) => self.publish(self.topic("info/volume"), v.to_string()).await,
            Err(e) => error!("get_volume failed: {e}"),
        }
    }

    async fn refresh_led_brightness(&self) {
        match self.speaker.get_led_brightness().await {
            Ok(v) => {
                self.publish(self.topic("info/led_brightness"), v.to_string()).await
            }
            Err(e) => error!("get_led_brightness failed: {e}"),
        }
    }

    async fn refresh_eq(&self) {
        match self.speaker.get_eq_profile().await {
            Ok(eq) => self.publish_eq(eq).await,
            Err(e) => error!("get_eq_profile failed: {e}"),
        }
    }

    async fn refresh_device_name(&self) {
        match self.speaker.get_device_name().await {
            Ok(name) => self.publish(self.topic("info/device_name"), name).await,
            Err(e) => error!("get_device_name failed: {e}"),
        }
    }

    // --- command (subscribe) handling -------------------------------------

    /// Handle an inbound message on `<prefix>/command/<suffix>`.
    pub async fn handle_command(&self, topic: &str, payload: &[u8]) {
        let command_prefix = format!("{}/command/", self.prefix);
        let Some(suffix) = topic.strip_prefix(&command_prefix) else {
            return;
        };
        let _guard = self.ble_lock.lock().await;
        let text = || String::from_utf8_lossy(payload).into_owned();

        match suffix {
            "set_volume" => {
                match text().trim().parse::<u8>() {
                    Ok(v) => self.run_set(self.speaker.set_volume(v), Self::refresh_volume).await,
                    Err(_) => error!("Bad set_volume value received"),
                }
            }
            "get_volume" => self.refresh_volume().await,

            "set_eq_preset" => match EqPreset::parse(text().trim()) {
                Some(p) => {
                    self.run_set(self.speaker.set_eq_preset(p), Self::refresh_eq).await
                }
                None => error!("Invalid set_eq_preset value received"),
            },
            "get_eq_preset" => self.refresh_eq().await,

            "set_eq_profile" => match parse_eq_profile(&text()) {
                Some(p) => {
                    self.run_set(self.speaker.set_eq_profile(p), Self::refresh_eq).await
                }
                None => error!("Invalid set_eq_profile value received"),
            },
            "get_eq_profile" => self.refresh_eq().await,

            "set_device_name" => {
                let name = text();
                self.run_set(self.speaker.set_device_name(&name), Self::refresh_device_name)
                    .await
            }
            "get_device_name" => self.refresh_device_name().await,

            "set_led_brightness" => match text().trim().parse::<u8>() {
                Ok(v) => {
                    self.run_set(self.speaker.set_led_brightness(v), Self::refresh_led_brightness)
                        .await
                }
                Err(_) => error!("Invalid set_led_brightness value received"),
            },
            "get_led_brightness" => self.refresh_led_brightness().await,

            "play" => self.run_cmd(self.speaker.play()).await,
            "pause" => self.run_cmd(self.speaker.pause()).await,
            "next" => self.run_cmd(self.speaker.next()).await,
            "previous" => self.run_cmd(self.speaker.previous()).await,

            "set_interaction_sound" => match text().trim() {
                "1" => self.run_set(self.speaker.set_interaction_sound(true), Self::refresh_status).await,
                "0" => self.run_set(self.speaker.set_interaction_sound(false), Self::refresh_status).await,
                _ => error!("Invalid set_interaction_sound value received"),
            },

            "get_status" => self.refresh_status().await,

            "set_source" => match AudioSource::parse(text().trim()) {
                Some(s) => self.run_set(self.speaker.set_source(s), Self::refresh_status).await,
                None => error!("Invalid set_source value received"),
            },

            "enter_pairing_mode" if self.allow_pairing => {
                if let Err(e) = self.speaker.enter_pairing_mode().await {
                    error!("enter_pairing_mode failed: {e}");
                } else {
                    warn!("Entered pairing mode; BLE will drop, exiting");
                    std::process::exit(0);
                }
            }

            _ if suffix.starts_with("set_eq_profile/") => {
                self.handle_individual_eq(suffix, &text()).await
            }

            other => warn!("Unhandled command: {other}"),
        }
    }

    async fn handle_individual_eq(&self, suffix: &str, value: &str) {
        let band = suffix
            .strip_prefix("set_eq_profile/")
            .and_then(|b| b.strip_suffix("hz"));
        let Some(band) = band else {
            error!("Invalid EQ band topic: {suffix}");
            return;
        };
        if !EQ_BANDS.contains(&band) {
            error!("Unknown EQ band: {band}");
            return;
        }
        let Ok(v) = value.trim().parse::<u8>() else {
            error!("Invalid set_eq_profile/{band}hz value received");
            return;
        };
        let mut profile = match self.speaker.get_eq_profile().await {
            Ok(p) => p,
            Err(e) => {
                error!("get_eq_profile failed: {e}");
                return;
            }
        };
        if profile.set_band(band, v).is_err() {
            error!("Invalid set_eq_profile/{band}hz value received");
            return;
        }
        self.run_set(self.speaker.set_eq_profile(profile), Self::refresh_eq).await;
    }

    /// Run a fire-and-forget command (play/pause/etc.) with no echo-back.
    async fn run_cmd(
        &self,
        fut: impl std::future::Future<Output = Result<(), crate::types::SpeakerError>>,
    ) {
        if let Err(e) = fut.await {
            error!("command failed: {e}");
        }
    }

    /// Run a set command, then (after a short delay) re-read and publish state so
    /// subscribers see the applied value.
    async fn run_set<'a, Fut, RefreshFut>(
        &'a self,
        fut: Fut,
        refresh: impl FnOnce(&'a Self) -> RefreshFut,
    ) where
        Fut: std::future::Future<Output = Result<(), crate::types::SpeakerError>>,
        RefreshFut: std::future::Future<Output = ()> + 'a,
    {
        match fut.await {
            Ok(()) => {
                tokio::time::sleep(REFRESH_DELAY).await;
                refresh(self).await;
            }
            Err(e) => error!("set command failed: {e}"),
        }
    }
}

fn parse_eq_profile(s: &str) -> Option<EqProfile> {
    let parts: Vec<&str> = s.split_whitespace().collect();
    if parts.len() != 5 {
        return None;
    }
    let mut vals = [0u8; 5];
    for (i, p) in parts.iter().enumerate() {
        vals[i] = p.parse::<u8>().ok()?;
    }
    EqProfile::from_slice(&vals).ok()
}

pub fn lwt_topic(prefix: &str) -> String {
    format!("{prefix}/lwt")
}

pub fn command_subscription(prefix: &str) -> String {
    format!("{prefix}/command/#")
}

pub fn log_startup(app: &App) {
    info!("MQTT control ready for prefix '{}'", app.prefix());
}
