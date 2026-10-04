//! Opt-in real-host regression: OpenDeck and Vincent Deck must already be running.
//! Exercises the physical encoder event path and verifies persisted selection.
use futures_util::SinkExt;
use serde_json::{Value, json};
use std::{path::PathBuf, time::Duration};
use tokio_tungstenite::tungstenite::Message;

#[tokio::test]
#[ignore = "changes the live device profile; requires running OpenDeck"]
async fn rightmost_click_switches_the_live_profile() {
    let device = "sd-EL31L1A08599";
    let config = PathBuf::from(std::env::var_os("HOME").unwrap()).join(".config/opendeck/profiles");
    let selection = config.join(format!("{device}.json"));
    let selected = || -> String {
        let data: Value = serde_json::from_slice(&std::fs::read(&selection).unwrap()).unwrap();
        data["selected_profile"].as_str().unwrap().into()
    };
    let before = selected();
    let expected = match before.as_str() {
        "performance" => "default",
        "default" => "performance",
        other => panic!("select default or performance before testing, got {other}"),
    };
    let profile: Value = serde_json::from_slice(
        &std::fs::read(config.join(device).join(format!("{before}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(
        profile["sliders"][3]["action"]["uuid"],
        if before == "default" {
            "dev.vincent.deck.theme"
        } else {
            "dev.vincent.deck.network"
        }
    );
    let port = std::env::var("OPENDECK_PORT").unwrap_or_else(|_| "57116".into());
    for event in ["encoderDown", "encoderUp"] {
        let (mut ws, _) = tokio_tungstenite::connect_async(format!("ws://127.0.0.1:{port}"))
            .await
            .unwrap();
        ws.send(Message::Text(
            json!({"event":event,"payload":{"device":device,"position":3}})
                .to_string()
                .into(),
        ))
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for _ in 0..30 {
        if selected() == expected {
            println!("PASS: rightmost click switched {before} -> {expected}");
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(
        selected(),
        expected,
        "dial click was delivered but live profile did not switch from {before}"
    );
}
