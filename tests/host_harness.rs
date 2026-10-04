use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    os::{fd::AsRawFd, unix::fs::PermissionsExt},
    process::Stdio,
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    net::{TcpListener, TcpStream},
    sync::watch,
    time::{sleep, timeout},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
mod common;

async fn send(
    ws: &mut WebSocketStream<TcpStream>,
    action: &str,
    context: &str,
    event: &str,
    ticks: i16,
) {
    let panel = [
        "volume",
        "mic",
        "workspace",
        "theme",
        "cpu",
        "memory",
        "disk",
        "network",
    ]
    .contains(&action);
    ws.send(Message::Text(json!({"event":event,"action":format!("dev.vincent.deck.{action}"),
        "context":context,"device":"test","payload":{"controller":if panel {"Encoder"} else {"Keypad"},
        "settings":{},"coordinates":{"row":0,"column":0},"isInMultiAction":false,
        "ticks":ticks,"pressed":false,"tapPos":[1,1],"hold":false}}).to_string().into())).await.unwrap();
}

async fn image(ws: &mut WebSocketStream<TcpStream>, context: &str) -> Value {
    timeout(Duration::from_secs(8), async {
        loop {
            let msg = ws.next().await.unwrap().unwrap();
            if let Message::Text(text) = msg {
                let value: Value = serde_json::from_str(&text).unwrap();
                if value["context"] == context
                    && ["setImage", "setFeedback"].contains(&value["event"].as_str().unwrap_or(""))
                {
                    return value;
                }
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn real_plugin_events_commands_duplicates_busy_gates_and_reconnect() {
    let temp = tempfile::tempdir().unwrap();
    let mut audio = common::PipeWire::start(temp.path());
    audio.start_policy();
    audio.control(&["set-volume", "@DEFAULT_AUDIO_SINK@", "120%"]);
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let state = temp.path().join(".local/state/omarchy/current");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("theme.name"), "matte-black\n").unwrap();
    let profiles = temp.path().join(".config/opendeck/profiles/test");
    std::fs::create_dir_all(&profiles).unwrap();
    for profile in ["default", "performance"] {
        std::fs::write(profiles.join(format!("{profile}.json")), "{}").unwrap();
    }
    let fixture = r#"#!/usr/bin/env python3
import os,sys,time,json
from pathlib import Path
root=Path(os.environ['HOME']); name=Path(sys.argv[0]).name; args=sys.argv[1:]
if name=='omarchy-theme-color':
 for k,v in [('bg',32),('darker_bg',16),('fg',235),('muted',155)]+[(k,195) for k in ['red','orange','yellow','green','cyan','blue','purple','magenta']]: print(k+'\t#'+('%02x'%v)*3)
elif name=='omarchy-theme-list': print('Matte Black')
elif name=='hyprctl':
 print({'workspaces':'[{"id":3,"windows":1}]','activeworkspace':'{"id":3}','activewindow':'{"class":"firefox","title":"test"}'}[args[0]])
elif name=='omarchy-toggle-nightlight' and args==['--status']: print('{"enabled":false,"temperature":null}')
elif name=='omarchy-powerprofiles-list':
 active=(root/'power').read_text() if (root/'power').exists() else 'performance'
 for profile in ['power-saver','balanced','performance']: print(profile+'\t'+str(int(profile==active)))
elif name=='omarchy-powerprofiles-set':
 with (root/'commands').open('a') as f: f.write(name+' '+' '.join(args)+'\n')
 time.sleep(.3)
 (root/'power').write_text(args[-1])
elif name=='omarchy-shell' and args==['lock','status']: print(json.dumps({'sessionLocked':(root/'locked').exists(),'pending':False}))
elif name=='pgrep': sys.exit(1)
else:
 with (root/'commands').open('a') as f: f.write(name+' '+' '.join(args)+'\n')
 if name in ['omarchy-toggle-nightlight','omarchy-capture-screenrecording','deck-workspace','deck-theme-cycle']: time.sleep(.7)
"#;
    let fixture_path = bin.join("fixture");
    std::fs::write(&fixture_path, fixture).unwrap();
    std::fs::set_permissions(&fixture_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    for name in [
        "omarchy-shell",
        "omarchy-powerprofiles-list",
        "omarchy-powerprofiles-set",
        "omarchy-theme-color",
        "omarchy-theme-list",
        "hyprctl",
        "omarchy-toggle-nightlight",
        "pgrep",
        "omarchy-launch-terminal",
        "omarchy-capture-screenrecording",
        "deck-workspace",
        "deck-theme-cycle",
        "omarchy-menu",
        "omarchy-theme-bg-next",
    ] {
        std::os::unix::fs::symlink("fixture", bin.join(name)).unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port().to_string();
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_vincent-deck"))
        .args([
            "-port",
            &port,
            "-pluginUUID",
            "dev.vincent.deck",
            "-registerEvent",
            "registerPlugin",
            "-info",
            "{\"devices\":[]}",
        ])
        .env("HOME", temp.path())
        .env("VINCENT_DECK_PATH", format!("{}:/usr/bin", bin.display()))
        .env("XDG_RUNTIME_DIR", temp.path())
        .env("PIPEWIRE_RUNTIME_DIR", temp.path())
        .env("PIPEWIRE_REMOTE", "vincent-test")
        .kill_on_drop(true)
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let stderr = child.stderr.take().unwrap();
    let (delivery_tx, mut delivery) = watch::channel(String::new());
    let logs = tokio::spawn(async move {
        let mut lines = BufReader::new(stderr).lines();
        while let Some(line) = lines.next_line().await.unwrap() {
            eprintln!("{line}");
            if line.starts_with("display stalled") || line.starts_with("display recovered") {
                delivery_tx.send_replace(line);
            }
        }
    });
    let (stream, _) = listener.accept().await.unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    let registration = ws.next().await.unwrap().unwrap();
    assert!(registration.to_text().unwrap().contains("registerPlugin"));
    for (action, context) in [
        ("lock", "l"),
        ("terminal", "a"),
        ("terminal", "b"),
        ("volume", "v"),
        ("night", "n"),
        ("night", "n2"),
        ("workspace", "w"),
        ("theme", "t"),
        ("cpu", "cpu"),
        ("memory", "memory"),
        ("disk", "disk"),
        ("network", "network"),
    ] {
        send(&mut ws, action, context, "willAppear", 0).await;
        let picture = image(&mut ws, context).await;
        let panel = [
            "volume",
            "workspace",
            "theme",
            "cpu",
            "memory",
            "disk",
            "network",
        ]
        .contains(&action);
        assert!(
            picture["payload"][if panel { "panel" } else { "image" }]
                .as_str()
                .unwrap()
                .starts_with("data:image/png;base64,")
        );
    }
    send(&mut ws, "terminal", "b", "keyDown", 0).await;
    send(&mut ws, "terminal", "b", "keyUp", 0).await;
    send(&mut ws, "volume", "v", "dialRotate", -1).await;
    for _ in 0..3 {
        send(&mut ws, "night", "n", "keyDown", 0).await;
        send(&mut ws, "night", "n2", "keyDown", 0).await;
    }
    send(&mut ws, "workspace", "w", "dialRotate", 4).await;
    send(&mut ws, "workspace", "w", "dialRotate", 1).await;
    send(&mut ws, "workspace", "w", "dialRotate", -2).await;
    for _ in 0..3 {
        send(&mut ws, "theme", "t", "dialRotate", 1).await;
    }
    send(&mut ws, "workspace", "w", "touchTap", 0).await;
    // CPU cycles available power modes; memory/disk and network rotation are inert.
    send(&mut ws, "cpu", "cpu", "dialRotate", 4).await;
    send(&mut ws, "cpu", "cpu", "dialRotate", 4).await;
    send(&mut ws, "cpu", "cpu", "dialDown", 0).await;
    send(&mut ws, "memory", "memory", "touchTap", 0).await;
    send(&mut ws, "disk", "disk", "dialRotate", 1).await;
    send(&mut ws, "network", "network", "dialRotate", 1).await;
    send(&mut ws, "network", "network", "touchTap", 0).await;
    send(&mut ws, "theme", "t", "touchTap", 0).await;
    for (action, context, target) in [
        ("theme", "t", "performance"),
        ("network", "network", "default"),
    ] {
        send(&mut ws, action, context, "dialDown", 0).await;
        let event = timeout(Duration::from_secs(3), async {
            loop {
                if let Message::Text(text) = ws.next().await.unwrap().unwrap() {
                    let event: Value = serde_json::from_str(&text).unwrap();
                    if event["event"] == "switchProfile" {
                        break event;
                    }
                }
            }
        })
        .await
        .unwrap();
        assert_eq!(event["device"], "test");
        assert_eq!(event["profile"], target);
        send(&mut ws, action, context, "dialUp", 0).await;
    }
    sleep(Duration::from_secs(2)).await;
    let log = std::fs::read_to_string(temp.path().join("commands")).unwrap();
    assert_eq!(log.matches("omarchy-launch-terminal").count(), 1, "{log}");
    assert_eq!(log.matches("omarchy-toggle-nightlight").count(), 1, "{log}");
    assert_eq!(log.matches("deck-theme-cycle").count(), 1, "{log}");
    assert_eq!(log.matches("deck-workspace").count(), 2, "{log}");
    assert!(log.contains("deck-workspace -1"));
    assert!(log.contains("omarchy-menu"));
    assert_eq!(
        log.matches("omarchy-theme-bg-next").count(),
        1,
        "dial click must not change wallpaper: {log}"
    );
    assert_eq!(log.matches("omarchy-powerprofiles-set").count(), 1, "{log}");
    assert!(log.contains("omarchy-powerprofiles-set autodetect power-saver"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("power")).unwrap(),
        "power-saver"
    );
    assert_eq!(
        log.matches("omarchy-shell shell summon omarchy.speedtest {}")
            .count(),
        1,
        "{log}"
    );
    let current = image(&mut ws, "cpu").await;
    let next = image(&mut ws, "cpu").await;
    assert_ne!(
        current["payload"], next["payload"],
        "CPU history must update over the host connection"
    );
    assert_eq!(
        audio.control(&["get-volume", "@DEFAULT_AUDIO_SINK@"]),
        "Volume: 1.00\n"
    );

    // Real backpressure: leave host receive half unread, rather than merely delaying
    // image consumption inside a test helper. Many active test surfaces fill TCP
    // buffers quickly; production surfaces still keep just one pending image each.
    audio.control(&["set-volume", "@DEFAULT_AUDIO_SINK@", "90%"]);
    let expected = image(&mut ws, "v").await;
    audio.control(&["set-volume", "@DEFAULT_AUDIO_SINK@", "100%"]);
    image(&mut ws, "v").await;
    let buffer: libc::c_int = 8192;
    // SAFETY: valid socket and correctly sized live integer pointer.
    assert_eq!(
        unsafe {
            libc::setsockopt(
                ws.get_ref().as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&buffer as *const libc::c_int).cast(),
                std::mem::size_of_val(&buffer) as libc::socklen_t,
            )
        },
        0
    );
    for index in 0..192 {
        send(&mut ws, "cpu", &format!("flood-{index}"), "willAppear", 0).await;
    }
    timeout(Duration::from_secs(20), async {
        loop {
            if delivery.borrow_and_update().starts_with("display stalled") {
                break;
            }
            delivery.changed().await.unwrap();
        }
    })
    .await
    .expect("must observe an actual blocked write, not assume TCP is full");
    // While sending is stalled, lifecycle events and local commands must complete.
    // Change volume several times; only final desired state should survive locally.
    for percent in ["30%", "50%", "90%"] {
        audio.control(&["set-volume", "@DEFAULT_AUDIO_SINK@", percent]);
        sleep(Duration::from_millis(150)).await;
    }
    for index in 0..192 {
        send(
            &mut ws,
            "cpu",
            &format!("flood-{index}"),
            "willDisappear",
            0,
        )
        .await;
    }
    send(&mut ws, "terminal", "a", "keyDown", 0).await;
    timeout(Duration::from_secs(3), async {
        loop {
            let log = std::fs::read_to_string(temp.path().join("commands")).unwrap();
            if log.matches("omarchy-launch-terminal").count() == 2 {
                break;
            }
            sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("display stall must not block input callbacks or lifecycle locks");
    assert!(child.try_wait().unwrap().is_none());
    let buffer: libc::c_int = 4 * 1024 * 1024;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                ws.get_ref().as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&buffer as *const libc::c_int).cast(),
                std::mem::size_of_val(&buffer) as libc::socklen_t,
            )
        },
        0
    );
    timeout(Duration::from_secs(8), async {
        loop {
            if image(&mut ws, "v").await["payload"] == expected["payload"] {
                break;
            }
        }
    })
    .await
    .expect("recovery must converge to newest audio state, tolerating buffered frames");
    // Explicit redraw after convergence establishes the current stable state too.
    send(&mut ws, "volume", "v", "willAppear", 0).await;
    assert_eq!(image(&mut ws, "v").await["payload"], expected["payload"]);
    timeout(Duration::from_secs(5), async {
        loop {
            if delivery
                .borrow_and_update()
                .starts_with("display recovered")
            {
                break;
            }
            delivery.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    send(&mut ws, "volume", "v", "dialDown", 0).await;
    send(&mut ws, "volume", "v", "dialUp", 0).await;
    sleep(Duration::from_millis(250)).await;
    assert_eq!(audio.default_name("default.audio.sink"), "alternate-output");
    assert!(
        !audio
            .control(&["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .contains("MUTED"),
        "dial click must not mute"
    );
    send(&mut ws, "volume", "v", "touchTap", 0).await;
    sleep(Duration::from_millis(250)).await;
    assert!(
        audio
            .control(&["get-volume", "@DEFAULT_AUDIO_SINK@"])
            .contains("MUTED"),
        "screen tap must mute"
    );
    send(&mut ws, "mic", "mic", "willAppear", 0).await;
    image(&mut ws, "mic").await;
    send(&mut ws, "mic", "mic", "dialDown", 0).await;
    sleep(Duration::from_millis(250)).await;
    assert_eq!(
        audio.default_name("default.audio.source"),
        "alternate-input"
    );
    assert!(
        !audio
            .control(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
            .contains("MUTED")
    );
    send(&mut ws, "mic", "mic", "touchTap", 0).await;
    sleep(Duration::from_millis(250)).await;
    assert!(
        audio
            .control(&["get-volume", "@DEFAULT_AUDIO_SOURCE@"])
            .contains("MUTED")
    );
    // Stop the private daemon: the real backend must render stale, remain alive,
    // then recover without replaying a failed adjustment.
    // Reading CPU frames may consume intervening volume frames; request a fresh
    // appearance instead of waiting for an unchanged volume image to be resent.
    send(&mut ws, "volume", "v", "willAppear", 0).await;
    let normal = image(&mut ws, "v").await;
    audio.stop();
    let stale = image(&mut ws, "v").await;
    assert_ne!(normal, stale);
    send(&mut ws, "volume", "v", "dialRotate", 5).await;
    sleep(Duration::from_millis(400)).await;
    audio = common::PipeWire::start(temp.path());
    image(&mut ws, "v").await;
    assert_eq!(
        audio.control(&["get-volume", "@DEFAULT_AUDIO_SINK@"]),
        "Volume: 1.00\n"
    );
    delivery.borrow_and_update();
    let buffer: libc::c_int = 8192;
    assert_eq!(
        unsafe {
            libc::setsockopt(
                ws.get_ref().as_raw_fd(),
                libc::SOL_SOCKET,
                libc::SO_RCVBUF,
                (&buffer as *const libc::c_int).cast(),
                std::mem::size_of_val(&buffer) as libc::socklen_t,
            )
        },
        0
    );
    for index in 0..192 {
        send(
            &mut ws,
            "cpu",
            &format!("close-flood-{index}"),
            "willAppear",
            0,
        )
        .await;
    }
    timeout(Duration::from_secs(20), async {
        loop {
            if delivery.borrow_and_update().starts_with("display stalled") {
                break;
            }
            delivery.changed().await.unwrap();
        }
    })
    .await
    .expect("disconnect test must start with an actual blocked SDK write");
    // Abrupt close must release the SDK writer lock on session end, not wait for
    // a websocket close frame to traverse the deliberately unread connection.
    drop(ws);
    let (stream, _) = timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
    ws.next().await.unwrap().unwrap();
    image(&mut ws, "a").await;
    child.kill().await.unwrap();
    child.wait().await.unwrap();
    logs.await.unwrap();
}
