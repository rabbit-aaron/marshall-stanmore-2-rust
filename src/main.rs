mod ble;
mod discovery;
mod mqtt;
mod types;

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use futures::StreamExt;
use rumqttc::{AsyncClient, Event, LastWill, MqttOptions, Packet, QoS};
use tracing::{error, info, warn};

use ble::Stanmore;
use mqtt::App;

struct Config {
    ble_address: String,
    mqtt_hostname: String,
    mqtt_port: u16,
    mqtt_username: Option<String>,
    mqtt_password: Option<String>,
    topic_prefix: String,
    retain: bool,
    allow_pairing: bool,
    discovery_prefix: String,
    discovery_enabled: bool,
}

fn env_opt(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.is_empty())
}

fn env_or(key: &str, default: &str) -> String {
    env_opt(key).unwrap_or_else(|| default.to_string())
}

fn env_bool(key: &str, default: bool) -> bool {
    match env_opt(key) {
        Some(v) => matches!(v.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        None => default,
    }
}

impl Config {
    fn from_env() -> Result<Self> {
        Ok(Self {
            ble_address: env_opt("BLE_ADDRESS")
                .context("BLE_ADDRESS environment variable must be set")?,
            mqtt_hostname: env_or("MQTT_HOSTNAME", "127.0.0.1"),
            mqtt_port: env_or("MQTT_PORT", "1883")
                .parse()
                .context("MQTT_PORT must be a valid port number")?,
            mqtt_username: env_opt("MQTT_USERNAME"),
            mqtt_password: env_opt("MQTT_PASSWORD"),
            topic_prefix: env_or("MQTT_TOPIC_PREFIX", "stanmore2"),
            retain: env_bool("MQTT_RETAIN", false),
            allow_pairing: env_bool("ALLOW_PAIRING", false),
            discovery_prefix: env_or("HA_DISCOVERY_PREFIX", "homeassistant"),
            discovery_enabled: env_bool("HA_DISCOVERY_ENABLED", true),
        })
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,btleplug=warn,rumqttc=warn".into()),
        )
        .init();

    let config = Config::from_env()?;

    let speaker = Stanmore::connect(&config.ble_address)
        .await
        .context("failed to connect to speaker over BLE")?;
    info!("Speaker connected");

    let node_id = format!("stanmore2_{}", discovery::sanitize(&config.ble_address));
    let client_id = node_id.clone();

    let mut opts = MqttOptions::new(client_id, &config.mqtt_hostname, config.mqtt_port);
    opts.set_keep_alive(Duration::from_secs(30));
    if let Some(user) = &config.mqtt_username {
        opts.set_credentials(user, config.mqtt_password.clone().unwrap_or_default());
    }
    opts.set_last_will(LastWill::new(
        mqtt::lwt_topic(&config.topic_prefix),
        "offline",
        QoS::AtLeastOnce,
        true,
    ));

    let (client, mut eventloop) = AsyncClient::new(opts, 128);

    let app = Arc::new(App::new(
        speaker,
        client.clone(),
        config.topic_prefix.clone(),
        config.retain,
        config.allow_pairing,
    ));
    mqtt::log_startup(&app);

    // Subscribe to commands and announce availability (queued; flushed once the
    // event loop establishes the connection below).
    client
        .subscribe(mqtt::command_subscription(&config.topic_prefix), QoS::AtLeastOnce)
        .await
        .context("failed to subscribe to command topic")?;
    app.publish_retain(mqtt::lwt_topic(&config.topic_prefix), "online", true)
        .await;

    if config.discovery_enabled {
        let messages =
            discovery::messages(&config.topic_prefix, &config.discovery_prefix, &node_id);
        info!("Publishing {} Home Assistant discovery configs", messages.len());
        for (topic, payload) in messages {
            app.publish_retain(topic, payload, true).await;
        }
    }

    // Push the current speaker state once connected.
    let state_app = app.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(1)).await;
        state_app.publish_all_state().await;
    });

    // BLE notification pump.
    let notif_app = app.clone();
    tokio::spawn(async move {
        let mut stream = match notif_app.notifications().await {
            Ok(s) => s,
            Err(e) => {
                error!("failed to open notification stream: {e}");
                std::process::exit(1);
            }
        };
        let mut media_buf = Vec::new();
        while let Some(n) = stream.next().await {
            if let Some(decoded) = Stanmore::decode_notification(n.uuid, &n.value, &mut media_buf) {
                notif_app.on_notification(decoded).await;
            }
        }
        error!("BLE notification stream ended (disconnected); exiting");
        std::process::exit(1);
    });

    // Graceful shutdown: publish offline then exit.
    let shutdown_app = app.clone();
    let shutdown_prefix = config.topic_prefix.clone();
    tokio::spawn(async move {
        wait_for_shutdown().await;
        warn!("Shutdown signal received");
        shutdown_app
            .publish_retain(mqtt::lwt_topic(&shutdown_prefix), "offline", true)
            .await;
        tokio::time::sleep(Duration::from_millis(200)).await;
        std::process::exit(0);
    });

    // Drive the MQTT event loop; dispatch inbound command messages.
    loop {
        match eventloop.poll().await {
            Ok(Event::Incoming(Packet::Publish(p))) => {
                let app = app.clone();
                let topic = p.topic.clone();
                let payload = p.payload.to_vec();
                tokio::spawn(async move {
                    app.handle_command(&topic, &payload).await;
                });
            }
            Ok(_) => {}
            Err(e) => {
                error!("MQTT connection error: {e}; retrying in 5s");
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
        }
    }
}

#[cfg(unix)]
async fn wait_for_shutdown() {
    use tokio::signal::unix::{signal, SignalKind};
    let mut term = signal(SignalKind::terminate()).expect("install SIGTERM handler");
    let mut int = signal(SignalKind::interrupt()).expect("install SIGINT handler");
    tokio::select! {
        _ = term.recv() => {}
        _ = int.recv() => {}
    }
}

#[cfg(not(unix))]
async fn wait_for_shutdown() {
    let _ = tokio::signal::ctrl_c().await;
}
