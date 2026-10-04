use crate::{
    process::Runner,
    render::{Action, Content, Palette, elapsed_label},
};
use anyhow::{Context, Result, bail};
use serde_json::Value;

#[derive(Clone, Default)]
pub struct Snapshot {
    pub palette: Palette,
    pub theme: Content,
    pub volume: Content,
    pub mic: Content,
    pub workspace: Content,
    pub night: Content,
    pub record: Content,
    pub lock: Content,
}

impl Snapshot {
    pub fn unavailable() -> Self {
        let stale = Content {
            stale: true,
            ..Default::default()
        };
        Self {
            theme: stale.clone(),
            volume: stale.clone(),
            mic: stale.clone(),
            workspace: stale.clone(),
            night: stale.clone(),
            record: stale.clone(),
            lock: stale,
            ..Default::default()
        }
    }

    /// The live content behind a stateful action; `None` for stateless keys.
    pub fn content(&self, action: Action) -> Option<&Content> {
        Some(match action {
            Action::Volume => &self.volume,
            Action::Mic => &self.mic,
            Action::Workspace => &self.workspace,
            Action::Theme => &self.theme,
            Action::Night => &self.night,
            Action::Record => &self.record,
            Action::Lock => &self.lock,
            _ => return None,
        })
    }

    pub fn content_mut(&mut self, action: Action) -> Option<&mut Content> {
        Some(match action {
            Action::Volume => &mut self.volume,
            Action::Mic => &mut self.mic,
            Action::Workspace => &mut self.workspace,
            Action::Theme => &mut self.theme,
            Action::Night => &mut self.night,
            Action::Record => &mut self.record,
            Action::Lock => &mut self.lock,
            _ => return None,
        })
    }
}

pub async fn audio(runner: &Runner, source: bool) -> Result<Content> {
    let target = if source {
        "@DEFAULT_AUDIO_SOURCE@"
    } else {
        "@DEFAULT_AUDIO_SINK@"
    };
    let output = runner.query(&["wpctl", "get-volume", target]).await?;
    let mut words = output.split_whitespace();
    if words.next() != Some("Volume:") {
        bail!("invalid wpctl volume {output:?}")
    }
    let value: f64 = words.next().context("missing volume")?.parse()?;
    if !value.is_finite() || value < 0.0 || value > u32::MAX as f64 / 100.0 {
        bail!("invalid volume")
    }
    Ok(Content {
        percent: Some((value * 100.0).round() as u32),
        muted: words.any(|s| s == "[MUTED]"),
        ..Default::default()
    })
}

pub async fn workspace(runner: &Runner) -> Result<Content> {
    let (all, active, window) = tokio::try_join!(
        runner.query(&["hyprctl", "workspaces", "-j"]),
        runner.query(&["hyprctl", "activeworkspace", "-j"]),
        runner.query(&["hyprctl", "activewindow", "-j"]),
    )?;
    let all: Value = serde_json::from_str(&all)?;
    let active: Value = serde_json::from_str(&active)?;
    let window: Value = serde_json::from_str(&window)?;
    let id = active["id"]
        .as_i64()
        .context("missing active workspace id")?;
    let mut content = Content {
        workspace: Some(i32::try_from(id)?),
        ..Default::default()
    };
    for ws in all.as_array().context("workspaces not array")? {
        let id = ws["id"].as_i64().context("workspace id")?;
        let windows = ws["windows"].as_u64().context("workspace windows")?;
        if (1..=10).contains(&id) {
            content.occupied[(id - 1) as usize] |= windows > 0;
        }
    }
    if !window.is_object() {
        bail!("activewindow not object")
    }
    let class = window["class"].as_str().unwrap_or("");
    let title = window["title"].as_str().unwrap_or("");
    // Bound stored text before rendering and keep application first.
    content.window = if class.is_empty() {
        String::new()
    } else if title.is_empty() {
        class.chars().take(64).collect()
    } else {
        format!(
            "{}: {}",
            class.chars().take(64).collect::<String>(),
            title.chars().take(64).collect::<String>()
        )
    };
    Ok(content)
}

pub async fn theme(runner: &Runner) -> Result<(Palette, Content)> {
    let name_path = runner.home.join(".local/state/omarchy/current/theme.name");
    let before = tokio::fs::read_to_string(&name_path).await?;
    let colors = runner.query(&["omarchy-theme-color", "--all"]).await?;
    let after = tokio::fs::read_to_string(&name_path).await?;
    if before != after {
        bail!("theme changed during palette read")
    }
    let palette = Palette::parse(&colors)?;
    let display = after
        .trim()
        .replace(['-', '_'], " ")
        .split_whitespace()
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|c| c.to_uppercase().collect::<String>() + chars.as_str())
                .unwrap_or_default()
        })
        .collect::<Vec<_>>()
        .join(" ");
    if display.is_empty() {
        bail!("empty theme name")
    }
    let list = runner.query(&["omarchy-theme-list"]).await?;
    let names: Vec<_> = list.lines().filter(|s| !s.trim().is_empty()).collect();
    let index = names
        .iter()
        .position(|s| s.to_lowercase().replace(['-', '_'], " ") == display.to_lowercase());
    Ok((
        palette,
        Content {
            theme: display.chars().take(64).collect(),
            position: index
                .map(|i| format!("{}/{}", i + 1, names.len()))
                .unwrap_or_else(|| format!("?/{}", names.len())),
            ..Default::default()
        },
    ))
}

pub async fn night(runner: &Runner) -> Result<Content> {
    let text = runner
        .query(&["omarchy-toggle-nightlight", "--status"])
        .await?;
    let value: Value = serde_json::from_str(&text)?;
    Ok(Content {
        active: value["enabled"]
            .as_bool()
            .context("night enabled missing")?,
        ..Default::default()
    })
}

pub async fn lock(runner: &Runner) -> Result<Content> {
    let text = runner.query(&["omarchy-shell", "lock", "status"]).await?;
    let value: Value = serde_json::from_str(&text)?;
    Ok(Content {
        active: value["sessionLocked"]
            .as_bool()
            .context("session lock status missing")?,
        pending: value["pending"]
            .as_bool()
            .context("pending lock status missing")?,
        ..Default::default()
    })
}

pub async fn record(runner: &Runner) -> Result<Content> {
    // pgrep exit 1 means confirmed idle, so capture status explicitly instead of
    // treating every query failure as idle.
    let argv = vec!["pgrep".into(), "-f".into(), "^gpu-screen-recorder".into()];
    let mut command = runner.command(&argv, false).await?;
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(2), command.output()).await??;
    if output.status.code() == Some(1) {
        return Ok(Content::default());
    }
    if !output.status.success() {
        bail!("pgrep exited {}", output.status)
    }
    let pids = String::from_utf8(output.stdout)?;
    let pid: u32 = pids
        .split_whitespace()
        .next()
        .context("pgrep returned no pid")?
        .parse()?;
    let elapsed = runner
        .query(&["ps", "-o", "etimes=", "-p", &pid.to_string()])
        .await?;
    let seconds: u64 = elapsed.trim().parse()?;
    Ok(Content {
        active: true,
        elapsed: elapsed_label(seconds),
        ..Default::default()
    })
}
