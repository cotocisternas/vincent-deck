mod common;

use std::time::Duration;
use tokio::sync::watch;
use vincent_deck::audio::{Audio, Level, State};

async fn wait(state: &mut watch::Receiver<State>, predicate: impl Fn(State) -> bool) -> State {
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let current = state.borrow_and_update().clone();
            if predicate(current.clone()) {
                return current;
            }
            state.changed().await.unwrap();
        }
    })
    .await
    .expect("native audio state did not converge")
}

#[tokio::test]
async fn native_controls_external_changes_defaults_and_reconnect_without_replay() {
    let temp = tempfile::tempdir().unwrap();
    let mut daemon = common::PipeWire::start(temp.path());
    daemon.start_policy();
    let audio = Audio::connect(temp.path().join("vincent-test").to_str().unwrap()).unwrap();
    let mut state = audio.subscribe();
    let initial = wait(&mut state, |s| s.output.is_some() && s.input.is_some()).await;
    assert_eq!(initial.output_device, "Test Speakers");
    assert_eq!(initial.input_device, "Test Mic");
    audio.cycle_device(false).await.unwrap();
    wait(&mut state, |s| s.output_device == "Headphones").await;
    assert_eq!(
        daemon.default_name("default.configured.audio.sink"),
        "alternate-output"
    );
    audio.cycle_device(false).await.unwrap();
    wait(&mut state, |s| s.output_device == "Test Speakers").await;
    audio.cycle_device(true).await.unwrap();
    wait(&mut state, |s| {
        s.input_device == "Headset Mic" && s.output_device == "Test Speakers"
    })
    .await;
    audio.cycle_device(true).await.unwrap();
    wait(&mut state, |s| s.input_device == "Test Mic").await;
    daemon.control(&["set-volume", "@DEFAULT_AUDIO_SINK@", "120%"]);
    wait(&mut state, |s| s.output.is_some_and(|l| l.percent == 120)).await;
    audio.adjust(false, None).await.unwrap();
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 120,
                muted: true,
            })
    })
    .await;
    assert!(
        daemon
            .control(&["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .contains("1.20 [MUTED]")
    );
    audio.adjust(false, Some(-1)).await.unwrap();
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 100,
                muted: true,
            })
    })
    .await;
    audio.adjust(false, None).await.unwrap();
    for _ in 0..10 {
        audio.adjust(false, Some(-1)).await.unwrap();
    }
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 90,
                muted: false,
            })
    })
    .await;
    assert_eq!(
        daemon.control(&["get-volume", "@DEFAULT_AUDIO_SINK@"]),
        "Volume: 0.90\n"
    );
    audio.adjust(true, Some(-25)).await.unwrap();
    audio.adjust(true, None).await.unwrap();
    wait(&mut state, |s| {
        s.input
            == Some(Level {
                percent: 75,
                muted: true,
            })
    })
    .await;
    assert_eq!(
        daemon.control(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"]),
        "Volume: 0.75 [MUTED]\n"
    );
    let result = daemon
        .command("pw-metadata")
        .args([
            "-n",
            "default",
            "0",
            "default.audio.sink",
            "{\"name\":\"alternate-output\"}",
            "Spa:String:JSON",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 100,
                muted: false,
            })
    })
    .await;
    audio.adjust(false, Some(-40)).await.unwrap();
    wait(&mut state, |s| s.output.is_some_and(|l| l.percent == 60)).await;
    let status = daemon.control(&["status", "--name"]);
    let id = status
        .lines()
        .find(|line| line.contains("alternate-output"))
        .unwrap()
        .split('.')
        .next()
        .unwrap()
        .split_whitespace()
        .last()
        .unwrap();
    daemon.control(&["set-default", id]);
    let removed = daemon
        .command("pw-cli")
        .args(["destroy", id])
        .output()
        .unwrap();
    assert!(removed.status.success());
    wait(&mut state, |s| s.output.is_none() && s.input.is_some()).await;
    audio.cycle_device(false).await.unwrap();
    wait(&mut state, |s| s.output_device == "Test Speakers").await;
    // A single remaining output is a harmless no-op.
    audio.cycle_device(false).await.unwrap();
    let result = daemon
        .command("pw-metadata")
        .args([
            "-n",
            "default",
            "0",
            "default.audio.sink",
            "{\"name\":\"missing\"}",
            "Spa:String:JSON",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    wait(&mut state, |s| s.output.is_none() && s.input.is_some()).await;
    assert!(audio.adjust(false, Some(5)).await.is_err());
    let result = daemon
        .command("pw-metadata")
        .args([
            "-n",
            "default",
            "0",
            "default.audio.sink",
            "{\"name\":\"test-output\"}",
            "Spa:String:JSON",
        ])
        .output()
        .unwrap();
    assert!(result.status.success());
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 90,
                muted: false,
            })
    })
    .await;
    daemon.stop();
    wait(&mut state, |s| s == State::default()).await;
    assert!(audio.adjust(false, Some(-10)).await.is_err());
    assert!(audio.cycle_device(true).await.is_err());
    daemon = common::PipeWire::start(temp.path());
    wait(&mut state, |s| {
        s.output
            == Some(Level {
                percent: 100,
                muted: false,
            })
            && s.input.is_some()
    })
    .await;
    assert_eq!(
        daemon.control(&["get-volume", "@DEFAULT_AUDIO_SINK@"]),
        "Volume: 1.00\n"
    );
}

#[tokio::test]
#[ignore = "reads the real desktop audio session; run separately from private-session tests"]
async fn live_native_audio_observation_matches_wpctl() {
    let audio = Audio::new();
    let mut state = audio.subscribe();
    let current = wait(&mut state, |s| s.output.is_some() && s.input.is_some()).await;
    for (target, level) in [
        ("@DEFAULT_AUDIO_SINK@", current.output.unwrap()),
        ("@DEFAULT_AUDIO_SOURCE@", current.input.unwrap()),
    ] {
        let output = std::process::Command::new("wpctl")
            .args(["get-volume", target])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let volume: f64 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
        assert_eq!(level.percent, (volume * 100.0).round() as u32);
        assert_eq!(level.muted, text.contains("[MUTED]"));
        println!("native {target}: {}% muted={}", level.percent, level.muted);
    }
    println!(
        "output device: {}; input device: {}",
        current.output_device, current.input_device
    );
}

#[tokio::test]
#[ignore = "cycles real desktop audio devices and restores the original defaults"]
async fn live_device_cycle_restores_defaults_and_preserves_levels() {
    fn metadata(key: &str) -> String {
        let output = std::process::Command::new("pw-metadata")
            .args(["-n", "default"])
            .output()
            .unwrap();
        assert!(output.status.success());
        let text = String::from_utf8(output.stdout).unwrap();
        let line = text
            .lines()
            .find(|line| line.contains(&format!("key:'{key}'")))
            .unwrap();
        line.split("value:'")
            .nth(1)
            .unwrap()
            .split('\'')
            .next()
            .unwrap()
            .into()
    }
    struct Restore(Vec<(&'static str, String)>);
    impl Drop for Restore {
        fn drop(&mut self) {
            for (key, value) in &self.0 {
                let _ = std::process::Command::new("pw-metadata")
                    .args(["-n", "default", "0", key, value, "Spa:String:JSON"])
                    .output();
            }
        }
    }
    let _restore = Restore(
        [
            "default.configured.audio.sink",
            "default.configured.audio.source",
        ]
        .into_iter()
        .map(|key| (key, metadata(key)))
        .collect(),
    );
    let audio = Audio::new();
    let mut state = audio.subscribe();
    let initial = wait(&mut state, |s| s.output.is_some() && s.input.is_some()).await;
    for (source, key) in [
        (false, "default.audio.sink"),
        (true, "default.audio.source"),
    ] {
        let original: serde_json::Value = serde_json::from_str(&metadata(key)).unwrap();
        let mut restored = false;
        for _ in 0..16 {
            audio.cycle_device(source).await.unwrap();
            let selected: serde_json::Value = serde_json::from_str(&metadata(key)).unwrap();
            println!("native device cycle {key}: {}", selected["name"]);
            if selected == original {
                restored = true;
                break;
            }
        }
        assert!(restored, "device cycle did not wrap to original default");
    }
    wait(&mut state, |s| s == initial).await;
}
