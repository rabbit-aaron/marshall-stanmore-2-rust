use serde_json::{json, Value};

use crate::types::{AudioSource, EqPreset};

/// Sanitize a string into a value safe for use in an MQTT topic / object id.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' })
        .collect()
}

/// Build the set of Home Assistant MQTT discovery messages: `(config_topic, payload)`.
///
/// Each entity points its command/state topics at the existing `<prefix>/command/*`
/// and `<prefix>/info/*` topics, ties itself to a shared `device`, and shares the
/// `<prefix>/lwt` availability topic so HA marks the speaker offline via the LWT.
pub fn messages(
    prefix: &str,
    discovery_prefix: &str,
    node_id: &str,
) -> Vec<(String, String)> {
    let availability = format!("{prefix}/lwt");
    let device = json!({
        "identifiers": [node_id],
        "name": "Marshall Stanmore II",
        "manufacturer": "Marshall",
        "model": "Stanmore II",
    });

    let cmd = |suffix: &str| format!("{prefix}/command/{suffix}");
    let info = |suffix: &str| format!("{prefix}/info/{suffix}");

    // (component, object_id, entity-specific config)
    let mut entities: Vec<(&str, String, Value)> = Vec::new();

    entities.push((
        "number",
        "volume".into(),
        json!({
            "name": "Volume",
            "command_topic": cmd("set_volume"),
            "state_topic": info("volume"),
            "min": 0, "max": 32, "step": 1,
            "icon": "mdi:volume-high",
        }),
    ));

    entities.push((
        "number",
        "led_brightness".into(),
        json!({
            "name": "LED brightness",
            "command_topic": cmd("set_led_brightness"),
            "state_topic": info("led_brightness"),
            "min": 0, "max": 35, "step": 1,
            "icon": "mdi:brightness-6",
        }),
    ));

    for (band, label) in [
        ("160", "EQ 160 Hz"),
        ("400", "EQ 400 Hz"),
        ("1000", "EQ 1 kHz"),
        ("2500", "EQ 2.5 kHz"),
        ("6250", "EQ 6.25 kHz"),
    ] {
        entities.push((
            "number",
            format!("eq_{band}hz"),
            json!({
                "name": label,
                "command_topic": cmd(&format!("set_eq_profile/{band}hz")),
                "state_topic": info(&format!("eq_profile/{band}hz")),
                "min": 0, "max": 10, "step": 1,
                "icon": "mdi:equalizer",
                "entity_category": "config",
            }),
        ));
    }

    let mut preset_options: Vec<&str> = EqPreset::ALL.iter().map(|p| p.as_str()).collect();
    preset_options.push("custom");
    entities.push((
        "select",
        "eq_preset".into(),
        json!({
            "name": "EQ preset",
            "command_topic": cmd("set_eq_preset"),
            "state_topic": info("eq_preset"),
            "options": preset_options,
            "icon": "mdi:equalizer",
        }),
    ));

    let source_options: Vec<&str> =
        [AudioSource::Bluetooth, AudioSource::Aux, AudioSource::Rca]
            .iter()
            .map(|s| s.as_str())
            .collect();
    entities.push((
        "select",
        "audio_source".into(),
        json!({
            "name": "Audio source",
            "command_topic": cmd("set_source"),
            "state_topic": info("audio_source"),
            "options": source_options,
            "icon": "mdi:audio-input-rca",
        }),
    ));

    entities.push((
        "switch",
        "interaction_sound".into(),
        json!({
            "name": "Interaction sound",
            "command_topic": cmd("set_interaction_sound"),
            "state_topic": info("interaction_sound_enabled"),
            "payload_on": "1",
            "payload_off": "0",
            "icon": "mdi:gesture-tap-button",
            "entity_category": "config",
        }),
    ));

    entities.push((
        "text",
        "device_name".into(),
        json!({
            "name": "Device name",
            "command_topic": cmd("set_device_name"),
            "state_topic": info("device_name"),
            "max": 17,
            "entity_category": "config",
        }),
    ));

    entities.push((
        "sensor",
        "play_status".into(),
        json!({
            "name": "Play status",
            "state_topic": info("play_status"),
            "icon": "mdi:play-pause",
        }),
    ));

    for (id, label, topic) in [
        ("media_title", "Media title", "media/title"),
        ("media_artist", "Media artist", "media/artist"),
        ("media_album", "Media album", "media/album"),
    ] {
        entities.push((
            "sensor",
            id.into(),
            json!({
                "name": label,
                "state_topic": info(topic),
                "icon": "mdi:music-note",
            }),
        ));
    }

    for (id, label, suffix, icon) in [
        ("play", "Play", "play", "mdi:play"),
        ("pause", "Pause", "pause", "mdi:pause"),
        ("next", "Next", "next", "mdi:skip-next"),
        ("previous", "Previous", "previous", "mdi:skip-previous"),
    ] {
        entities.push((
            "button",
            id.into(),
            json!({
                "name": label,
                "command_topic": cmd(suffix),
                "payload_press": "",
                "icon": icon,
            }),
        ));
    }

    entities
        .into_iter()
        .map(|(component, object_id, mut config)| {
            let obj = config.as_object_mut().unwrap();
            obj.insert("unique_id".into(), json!(format!("{node_id}_{object_id}")));
            obj.insert("availability_topic".into(), json!(availability));
            obj.insert("payload_available".into(), json!("online"));
            obj.insert("payload_not_available".into(), json!("offline"));
            obj.insert("device".into(), device.clone());
            let topic = format!("{discovery_prefix}/{component}/{node_id}/{object_id}/config");
            (topic, config.to_string())
        })
        .collect()
}
