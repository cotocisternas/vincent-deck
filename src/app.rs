use crate::{
    process::Runner,
    render::{Action, Content, Palette, Renderer},
    state::{self, Snapshot},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use std::{
    collections::HashMap,
    future::Future,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    sync::{Mutex, Notify},
    time::sleep,
};

pub struct App {
    pub runner: Runner,
    renderer: Renderer,
    snapshot: Mutex<Snapshot>,
    surfaces: Mutex<HashMap<String, Surface>>,
    gates: Mutex<HashMap<Action, Gate>>,
    audio_locks: [Mutex<()>; 2],
    audio_pending: Mutex<HashMap<Action, AudioPending>>,
    notify: Notify,
}
struct Surface {
    action: Action,
    instance: Arc<openaction::Instance>,
    last: String,
    sent: Instant,
    failed_until: Option<Instant>,
}
#[derive(Default)]
struct Gate {
    busy: bool,
    pending: Option<(i16, String)>,
}
#[derive(Default)]
struct AudioPending {
    running: bool,
    ticks: i64,
    mute: bool,
    origin: String,
}

impl App {
    pub fn new(renderer: Renderer) -> Arc<Self> {
        Arc::new(Self {
            runner: Runner::new(),
            renderer,
            snapshot: Mutex::new(Snapshot::unavailable()),
            surfaces: Mutex::new(HashMap::new()),
            gates: Mutex::new(HashMap::new()),
            audio_locks: [Mutex::new(()), Mutex::new(())],
            audio_pending: Mutex::new(HashMap::new()),
            notify: Notify::new(),
        })
    }

    pub async fn appear(&self, action: Action, instance: &openaction::Instance) {
        if instance.controller != if action.panel() { "Encoder" } else { "Keypad" } {
            return;
        }
        if let Some(instance) = openaction::get_instance(instance.instance_id.clone()).await {
            self.surfaces.lock().await.insert(
                instance.instance_id.clone(),
                Surface {
                    action,
                    instance,
                    last: String::new(),
                    sent: Instant::now() - Duration::from_secs(1),
                    failed_until: None,
                },
            );
            self.notify.notify_one();
        }
    }
    pub async fn disappear(&self, id: &str) {
        self.surfaces.lock().await.remove(id);
    }
    pub async fn reconnect(&self) {
        for surface in self.surfaces.lock().await.values_mut() {
            surface.last.clear();
        }
        self.notify.notify_one();
    }

    pub async fn disconnect_device(&self, device: &str) {
        self.surfaces
            .lock()
            .await
            .retain(|_, surface| surface.instance.device_id != device);
    }
    async fn failure(&self, id: &str, error: impl std::fmt::Display) {
        eprintln!("command failure context={id}: {error}");
        if let Some(surface) = self.surfaces.lock().await.get_mut(id) {
            surface.failed_until = Some(Instant::now() + Duration::from_secs(2));
        }
        self.notify.notify_one();
    }

    pub fn start(self: &Arc<Self>) {
        self.spawn(|app| async move { app.render_loop().await });
        for action in POLLED {
            self.spawn(move |app| async move { app.poll_loop(action).await });
        }
        self.spawn(|app| async move { app.audio_events().await });
        self.spawn(|app| async move { app.hypr_events().await });
    }

    fn spawn<F, Fut>(self: &Arc<Self>, task: F)
    where
        F: FnOnce(Arc<Self>) -> Fut,
        Fut: Future<Output = ()> + Send + 'static,
    {
        tokio::spawn(task(self.clone()));
    }

    // ---- polling -------------------------------------------------------

    async fn poll_loop(&self, action: Action) {
        let mut theme = ThemeWatch::new();
        let mut retry_delay = 500_u64;
        loop {
            if action == Action::Theme && !self.theme_due(&mut theme).await {
                sleep(Duration::from_millis(250)).await;
                continue;
            }
            self.refresh(action).await;
            let (stale, active) = {
                let snapshot = self.snapshot.lock().await;
                let content = snapshot.content(action);
                (
                    content.is_some_and(|c| c.stale),
                    content.is_some_and(|c| c.active),
                )
            };
            let delay = next_poll_delay(stale, poll_interval(action, active), &mut retry_delay);
            sleep(Duration::from_millis(delay)).await;
        }
    }

    /// Whether the theme needs re-reading. Reads through replacement of the
    /// current directory; an inode watch would stop observing it after theme changes.
    async fn theme_due(&self, watch: &mut ThemeWatch) -> bool {
        let root = self.runner.home.join(".local/state/omarchy/current");
        let name = tokio::fs::read_to_string(root.join("theme.name"))
            .await
            .unwrap_or_default();
        let colors = tokio::fs::metadata(root.join("theme/colors.toml"))
            .await
            .ok()
            .and_then(|m| m.modified().ok());
        let fingerprint = format!("{name}:{colors:?}");
        let changed = fingerprint != watch.fingerprint;
        if !changed
            && watch.checked.elapsed() < Duration::from_secs(5)
            && !self.snapshot.lock().await.theme.stale
        {
            return false;
        }
        if changed {
            sleep(Duration::from_millis(400)).await;
            watch.fingerprint = fingerprint;
        }
        watch.checked = Instant::now();
        true
    }

    // ---- refresh -------------------------------------------------------

    async fn refresh(&self, action: Action) {
        // Serialize reads with audio writes to prevent older snapshots overwriting
        // a just-completed adjustment.
        let _audio_guard = match audio_lane(action) {
            Some(lane) => Some(self.audio_locks[lane].lock().await),
            None => None,
        };
        if action == Action::Theme {
            self.refresh_theme().await;
        } else if !self.refresh_content(action).await {
            return;
        }
        self.notify.notify_one();
    }

    async fn refresh_theme(&self) {
        let result = state::theme(&self.runner).await;
        let mut s = self.snapshot.lock().await;
        match result {
            Ok((palette, content)) => {
                s.palette = palette;
                s.theme = content;
            }
            Err(error) => flag_stale(&mut s.theme, "theme", &error),
        }
    }

    /// Returns false when the action has no polled state.
    async fn refresh_content(&self, action: Action) -> bool {
        let Some(result) = self.query_state(action).await else {
            return false;
        };
        let mut s = self.snapshot.lock().await;
        if let Some(target) = s.content_mut(action) {
            match result {
                Ok(content) => *target = content,
                Err(error) => flag_stale(target, action.name(), &error),
            }
        }
        true
    }

    async fn query_state(&self, action: Action) -> Option<anyhow::Result<Content>> {
        Some(match action {
            Action::Volume => state::audio(&self.runner, false).await,
            Action::Mic => state::audio(&self.runner, true).await,
            Action::Workspace => state::workspace(&self.runner).await,
            Action::Night => state::night(&self.runner).await,
            Action::Record => state::record(&self.runner).await,
            Action::Lock => state::lock(&self.runner).await,
            _ => return None,
        })
    }

    async fn mark_stale(&self, actions: &[Action]) {
        {
            let mut s = self.snapshot.lock().await;
            for action in actions {
                if let Some(content) = s.content_mut(*action) {
                    content.stale = true;
                }
            }
        }
        self.notify.notify_one();
    }

    // ---- rendering -----------------------------------------------------

    async fn render_loop(&self) {
        // A fixed 100 ms cadence bounds latency even during sustained events and
        // enforces <=10 Hz per panel; notification only wakes an idle loop.
        let mut cache = RenderCache::default();
        loop {
            self.notify.notified().await;
            loop {
                let pending = self.render_pass(&mut cache).await;
                sleep(Duration::from_millis(100)).await;
                if !pending && !self.notify.notified().now_or_never_ready() {
                    break;
                }
            }
        }
    }

    /// Renders and pushes every surface once; returns whether another pass is needed.
    async fn render_pass(&self, cache: &mut RenderCache) -> bool {
        let snapshot = self.snapshot.lock().await.clone();
        let gates = self.gates.lock().await;
        let mut surfaces = self.surfaces.lock().await;
        let mut pending = false;
        for surface in surfaces.values_mut() {
            let content = surface.content(&snapshot, &gates);
            pending |= content.failed;
            let Some(uri) = cache.uri(&self.renderer, surface.action, &snapshot.palette, content)
            else {
                continue;
            };
            pending |= surface.push(uri).await;
        }
        pending
    }

    // ---- input ---------------------------------------------------------

    pub async fn input(self: &Arc<Self>, action: Action, id: String, ticks: Option<i16>) {
        if ticks == Some(0) {
            return;
        }
        if audio_lane(action).is_some() {
            self.queue_audio(action, id, ticks).await;
            return;
        }
        let gated = matches!(action, Action::Night | Action::Record)
            || (ticks.is_some() && matches!(action, Action::Workspace | Action::Theme));
        if gated && !self.claim_gate(action, ticks, &id).await {
            return;
        }
        self.spawn(move |app| async move { app.run_command(action, ticks, id, gated).await });
    }

    /// Takes the action's gate; while busy, Workspace keeps only its latest direction.
    async fn claim_gate(&self, action: Action, ticks: Option<i16>, id: &str) -> bool {
        let mut gates = self.gates.lock().await;
        let gate = gates.entry(action).or_default();
        if gate.busy {
            if action == Action::Workspace {
                gate.pending = Some((ticks.unwrap().signum(), id.to_string()));
            }
            return false;
        }
        gate.busy = true;
        true
    }

    /// Next coalesced request, or releases the gate when there is none.
    async fn next_pending(&self, action: Action) -> Option<(Option<i16>, String)> {
        let mut gates = self.gates.lock().await;
        let gate = gates.get_mut(&action).unwrap();
        if let Some((ticks, id)) = gate.pending.take() {
            return Some((Some(ticks), id));
        }
        gate.busy = false;
        None
    }

    async fn run_command(&self, action: Action, ticks: Option<i16>, id: String, gated: bool) {
        let mut current = (ticks, id);
        loop {
            self.notify.notify_one();
            self.execute(action, current.0, &current.1).await;
            if !gated {
                return;
            }
            match self.next_pending(action).await {
                Some(next) => current = next,
                None => break,
            }
        }
        self.notify.notify_one();
        if matches!(action, Action::Night | Action::Record) {
            for delay in [0, 300, 700, 1500] {
                sleep(Duration::from_millis(delay)).await;
                self.refresh(action).await;
            }
        }
    }

    async fn execute(&self, action: Action, ticks: Option<i16>, id: &str) {
        let argv = command(action, ticks);
        eprintln!("command context={id} argv={argv:?}");
        if let Err(error) = self.run_to_completion(&argv).await {
            self.failure(id, error).await;
        }
    }

    async fn run_to_completion(&self, argv: &[String]) -> anyhow::Result<()> {
        let mut child = self.runner.launch(argv).await?;
        let status = child.wait().await?;
        anyhow::ensure!(status.success(), "exit {status}");
        Ok(())
    }

    // ---- audio ---------------------------------------------------------

    /// Folds the request into the action's pending batch; one worker drains it.
    async fn queue_audio(self: &Arc<Self>, action: Action, id: String, ticks: Option<i16>) {
        {
            let mut queue = self.audio_pending.lock().await;
            let pending = queue.entry(action).or_default();
            pending.origin = id;
            match ticks {
                Some(ticks) => pending.ticks = pending.ticks.saturating_add(ticks as i64),
                None => pending.mute = !pending.mute,
            }
            if pending.running {
                return;
            }
            pending.running = true;
        }
        self.spawn(move |app| async move { app.drain_audio(action).await });
    }

    async fn drain_audio(&self, action: Action) {
        // Requests already accepted during a failed command must not
        // survive backend recovery. The next batch runs immediately.
        while let Some((ticks, mute, origin)) = self.take_audio_batch(action).await {
            if ticks != 0 {
                self.audio_input(action, &origin, Some(ticks)).await;
            }
            if mute {
                self.audio_input(action, &origin, None).await;
            }
        }
    }

    async fn take_audio_batch(&self, action: Action) -> Option<(i64, bool, String)> {
        let mut queue = self.audio_pending.lock().await;
        let pending = queue.get_mut(&action).unwrap();
        let ticks = std::mem::take(&mut pending.ticks);
        let mute = std::mem::take(&mut pending.mute);
        if ticks == 0 && !mute {
            pending.running = false;
            return None;
        }
        Some((ticks, mute, pending.origin.clone()))
    }

    async fn audio_input(&self, action: Action, id: &str, ticks: Option<i64>) {
        if ticks == Some(0) {
            return;
        }
        let source = action == Action::Mic;
        let _guard = self.audio_locks[usize::from(source)].lock().await;
        let result = self.apply_audio(source, ticks).await;
        let failure = {
            let mut s = self.snapshot.lock().await;
            let content = if source { &mut s.mic } else { &mut s.volume };
            match result {
                Ok(value) => {
                    *content = value;
                    None
                }
                Err(error) => {
                    content.stale = true;
                    Some(error)
                }
            }
        };
        match failure {
            Some(error) => self.failure(id, error).await,
            None => self.notify.notify_one(),
        }
    }

    /// Reads the device, applies the volume step or mute toggle, and returns the new content.
    async fn apply_audio(&self, source: bool, ticks: Option<i64>) -> anyhow::Result<Content> {
        let target = if source {
            "@DEFAULT_AUDIO_SOURCE@"
        } else {
            "@DEFAULT_AUDIO_SINK@"
        };
        let mut content = state::audio(&self.runner, source).await?;
        if let Some(ticks) = ticks {
            let value = (content.percent.unwrap() as i64)
                .saturating_add(ticks)
                .clamp(0, 100);
            self.runner
                .query(&["wpctl", "set-volume", target, &format!("{value}%")])
                .await?;
            content.percent = Some(value as u32);
        } else {
            self.runner
                .query(&["wpctl", "set-mute", target, "toggle"])
                .await?;
            content.muted = !content.muted;
        }
        Ok(content)
    }

    // ---- backend event streams -----------------------------------------

    async fn audio_events(&self) {
        let mut delay = 1;
        loop {
            let received = self.watch_audio().await;
            self.mark_stale(&[Action::Volume, Action::Mic]).await;
            reconnect_pause(&mut delay, received).await;
        }
    }

    /// Follows `pactl subscribe` until it ends; returns whether any event arrived.
    async fn watch_audio(&self) -> bool {
        let argv = vec!["pactl".into(), "subscribe".into()];
        let Ok(mut command) = self.runner.command(&argv, false).await else {
            return false;
        };
        command
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let Ok(mut child) = command.spawn() else {
            return false;
        };
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        let mut received = false;
        while let Ok(Some(line)) = lines.next_line().await {
            if ["sink", "source", "server"]
                .iter()
                .any(|s| line.contains(s))
            {
                self.refresh(Action::Volume).await;
                self.refresh(Action::Mic).await;
            }
            received = true;
        }
        let _ = child.kill().await;
        let _ = child.wait().await;
        received
    }

    async fn hypr_events(&self) {
        let mut delay = 1;
        loop {
            let received = self.watch_hypr().await;
            self.mark_stale(&[Action::Workspace]).await;
            reconnect_pause(&mut delay, received).await;
        }
    }

    async fn hypr_socket(&self) -> Option<tokio::net::UnixStream> {
        let sig = self.runner.signature().await?;
        let runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
        tokio::net::UnixStream::connect(format!("{runtime}/hypr/{sig}/.socket2.sock"))
            .await
            .ok()
    }

    /// Follows Hyprland's event socket until it closes; returns whether any event arrived.
    async fn watch_hypr(&self) -> bool {
        let Some(socket) = self.hypr_socket().await else {
            return false;
        };
        let mut lines = BufReader::new(socket).lines();
        self.refresh(Action::Workspace).await;
        let mut received = false;
        while let Ok(Some(_)) = lines.next_line().await {
            self.refresh(Action::Workspace).await;
            received = true;
        }
        received
    }
}

/// Actions whose state is polled in the background.
const POLLED: [Action; 7] = [
    Action::Volume,
    Action::Mic,
    Action::Workspace,
    Action::Theme,
    Action::Night,
    Action::Record,
    Action::Lock,
];

/// Index of the audio lock/queue lane for an audio action.
fn audio_lane(action: Action) -> Option<usize> {
    match action {
        Action::Volume => Some(0),
        Action::Mic => Some(1),
        _ => None,
    }
}

fn poll_interval(action: Action, active: bool) -> u64 {
    match action {
        Action::Workspace => 2000,
        Action::Record if active => 1000,
        Action::Record | Action::Night => 2000,
        _ => 500,
    }
}

/// Normal cadence while healthy; doubling backoff (capped at 10 s) while stale.
fn next_poll_delay(stale: bool, normal: u64, retry_delay: &mut u64) -> u64 {
    if stale {
        *retry_delay = (*retry_delay * 2).min(10_000);
        *retry_delay
    } else {
        *retry_delay = 500;
        normal
    }
}

/// Backoff between event-stream reconnects, reset by any received event (capped at 30 s).
async fn reconnect_pause(delay: &mut u64, received: bool) {
    if received {
        *delay = 1;
    }
    sleep(Duration::from_secs(*delay)).await;
    *delay = (*delay * 2).min(30);
}

fn flag_stale(content: &mut Content, name: &str, error: &anyhow::Error) {
    if !content.stale {
        eprintln!("{name} stale: {error:#}");
    }
    content.stale = true;
}

struct ThemeWatch {
    fingerprint: String,
    checked: Instant,
}
impl ThemeWatch {
    fn new() -> Self {
        Self {
            fingerprint: String::new(),
            checked: Instant::now() - Duration::from_secs(10),
        }
    }
}

impl Surface {
    /// The content to draw: live state plus this surface's transient flags.
    fn content(&self, snapshot: &Snapshot, gates: &HashMap<Action, Gate>) -> Content {
        let mut content = snapshot.content(self.action).cloned().unwrap_or_default();
        content.theme_stale = snapshot.theme.stale;
        content.pending |= matches!(self.action, Action::Night | Action::Record)
            && gates.get(&self.action).is_some_and(|g| g.busy);
        content.failed = self.failed_until.is_some_and(|t| t > Instant::now());
        content
    }

    /// Sends the image if it changed; returns whether the surface needs another pass.
    async fn push(&mut self, uri: String) -> bool {
        if uri == self.last {
            return false;
        }
        let panel = self.action.panel();
        if panel && self.sent.elapsed() < Duration::from_millis(100) {
            return true;
        }
        let result = if panel {
            self.instance
                .set_feedback(&serde_json::json!({"panel": uri}))
                .await
        } else {
            self.instance.set_image(Some(&uri), Some(0)).await
        };
        match result {
            Ok(()) => {
                self.last = uri;
                self.sent = Instant::now();
                false
            }
            Err(error) => {
                eprintln!("display: {error}");
                true
            }
        }
    }
}

/// Bounded cache of rendered data URIs keyed by visible content.
#[derive(Default)]
struct RenderCache(Vec<(Action, Palette, Content, String)>);
impl RenderCache {
    fn uri(
        &mut self,
        renderer: &Renderer,
        action: Action,
        palette: &Palette,
        content: Content,
    ) -> Option<String> {
        if let Some(entry) = self
            .0
            .iter()
            .find(|(a, p, c, _)| *a == action && p == palette && *c == content)
        {
            return Some(entry.3.clone());
        }
        match renderer.render(action, palette, &content) {
            Ok(png) => {
                let uri = format!("data:image/png;base64,{}", STANDARD.encode(png));
                if self.0.len() >= 64 {
                    self.0.remove(0);
                }
                self.0.push((action, palette.clone(), content, uri.clone()));
                Some(uri)
            }
            Err(error) => {
                eprintln!("render failure: {error:#}");
                None
            }
        }
    }
}

fn command(action: Action, ticks: Option<i16>) -> Vec<String> {
    let args: Vec<&str> = match action {
        Action::Terminal => vec!["omarchy-launch-terminal"],
        Action::Browser => vec!["omarchy-launch-browser"],
        Action::Screenshot => vec!["omarchy-capture-screenshot"],
        Action::Record => vec!["omarchy-capture-screenrecording"],
        Action::Agent => vec![
            "hyprctl",
            "dispatch",
            "hl.dsp.workspace.toggle_special(\"scratchpad\")",
        ],
        Action::Clipboard => vec!["omarchy-menu-clipboard"],
        Action::Night => vec!["omarchy-toggle-nightlight"],
        Action::Lock => vec!["omarchy-system-lock"],
        Action::Workspace if ticks.is_some() => vec!["deck-workspace"],
        Action::Theme if ticks.is_some() => vec!["deck-theme-cycle"],
        Action::Workspace => vec!["omarchy-menu"],
        Action::Theme => vec!["omarchy-theme-bg-next"],
        _ => vec![],
    };
    let mut args = args.into_iter().map(String::from).collect::<Vec<_>>();
    if let Some(ticks) = ticks {
        args.push(ticks.signum().to_string());
    }
    args
}

// Polling a Notify future once consumes an existing permit without waiting.
trait Ready {
    fn now_or_never_ready(self) -> bool;
}
impl<F: Future> Ready for F {
    fn now_or_never_ready(self) -> bool {
        use std::task::{Context, Poll, Waker};
        let mut future = std::pin::pin!(self);
        matches!(
            future
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Ready(_)
        )
    }
}
