use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{os::unix::fs::PermissionsExt, time::Duration};
use tokio::{
    net::{TcpListener, TcpStream},
    time::{sleep, timeout},
};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};

async fn send(
    ws: &mut WebSocketStream<TcpStream>,
    action: &str,
    context: &str,
    event: &str,
    ticks: i16,
) {
    let panel = ["volume", "mic", "workspace", "theme"].contains(&action);
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
    let bin = temp.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let state = temp.path().join(".local/state/omarchy/current");
    std::fs::create_dir_all(&state).unwrap();
    std::fs::write(state.join("theme.name"), "matte-black\n").unwrap();
    let fixture = r#"#!/usr/bin/env python3
import os,sys,time,json
from pathlib import Path
root=Path(os.environ['HOME']); name=Path(sys.argv[0]).name; args=sys.argv[1:]
if name=='wpctl':
 p=root/'volume'; v=float(p.read_text()) if p.exists() else 1.2
 if args[0]=='get-volume': print('Volume:',v)
 elif args[0]=='set-volume': p.write_text(str(float(args[-1].strip('%'))/100))
 else: pass
elif name=='omarchy-theme-color':
 for k,v in [('bg',32),('darker_bg',16),('fg',235),('muted',155)]+[(k,195) for k in ['red','orange','yellow','green','cyan','blue','purple','magenta']]: print(k+'\t#'+('%02x'%v)*3)
elif name=='omarchy-theme-list': print('Matte Black')
elif name=='hyprctl':
 print({'workspaces':'[{"id":3,"windows":1}]','activeworkspace':'{"id":3}','activewindow':'{"class":"firefox","title":"test"}'}[args[0]])
elif name=='omarchy-toggle-nightlight' and args==['--status']: print('{"enabled":false,"temperature":null}')
elif name=='omarchy-shell': print(json.dumps({'sessionLocked':(root/'locked').exists(),'pending':False}))
elif name=='pgrep': sys.exit(1)
elif name=='pactl': time.sleep(100)
else:
 with (root/'commands').open('a') as f: f.write(name+' '+' '.join(args)+'\n')
 if name in ['omarchy-toggle-nightlight','omarchy-capture-screenrecording','deck-workspace','deck-theme-cycle']: time.sleep(.7)
"#;
    let fixture_path = bin.join("fixture");
    std::fs::write(&fixture_path, fixture).unwrap();
    std::fs::set_permissions(&fixture_path, std::fs::Permissions::from_mode(0o755)).unwrap();
    for name in [
        "omarchy-shell",
        "wpctl",
        "omarchy-theme-color",
        "omarchy-theme-list",
        "hyprctl",
        "omarchy-toggle-nightlight",
        "pgrep",
        "pactl",
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
        .kill_on_drop(true)
        .spawn()
        .unwrap();
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
    ] {
        send(&mut ws, action, context, "willAppear", 0).await;
        let picture = image(&mut ws, context).await;
        assert!(picture["payload"][if action=="volume" || action=="workspace" || action=="theme" {"panel"} else {"image"}].as_str().unwrap().starts_with("data:image/png;base64,"));
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
    sleep(Duration::from_secs(2)).await;
    let log = std::fs::read_to_string(temp.path().join("commands")).unwrap();
    assert_eq!(log.matches("omarchy-launch-terminal").count(), 1, "{log}");
    assert_eq!(log.matches("omarchy-toggle-nightlight").count(), 1, "{log}");
    assert_eq!(log.matches("deck-theme-cycle").count(), 1, "{log}");
    assert_eq!(log.matches("deck-workspace").count(), 2, "{log}");
    assert!(log.contains("deck-workspace -1"));
    assert!(log.contains("omarchy-menu"));
    assert_eq!(
        std::fs::read_to_string(temp.path().join("volume")).unwrap(),
        "1.0"
    );
    // Remove a required tool: the real backend must render stale, remain alive,
    // then recover without replaying a failed adjustment.
    let normal = image(&mut ws, "v").await;
    std::fs::remove_file(bin.join("wpctl")).unwrap();
    let stale = image(&mut ws, "v").await;
    assert_ne!(normal, stale);
    send(&mut ws, "volume", "v", "dialRotate", 5).await;
    sleep(Duration::from_millis(400)).await;
    std::os::unix::fs::symlink("fixture", bin.join("wpctl")).unwrap();
    image(&mut ws, "v").await;
    assert_eq!(
        std::fs::read_to_string(temp.path().join("volume")).unwrap(),
        "1.0"
    );
    ws.close(None).await.unwrap();
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
}
