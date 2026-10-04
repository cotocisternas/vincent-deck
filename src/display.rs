//! Latest-only display delivery. No caller waits for socket I/O.
use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Notify, oneshot, watch},
    time::{Instant, sleep},
};

const CADENCE: Duration = Duration::from_millis(100);
const DEADLINE: Duration = Duration::from_secs(2);
const COMMAND_LIMIT: usize = 8;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Offline,
    Ready,
    Sending,
    Stalled,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Surface {
    id: String,
    generation: u64,
}

struct Entry {
    surface: Surface,
    panel: bool,
    desired: Option<Arc<str>>,
    last: Option<Arc<str>>,
    due: Instant,
    queued: bool,
    sending: bool,
}
struct Command {
    surface: Surface,
    event: serde_json::Value,
    expires: Instant,
    reply: Option<oneshot::Sender<Result<(), String>>>,
}
#[derive(Default)]
struct State {
    entries: HashMap<String, Entry>,
    dirty: VecDeque<Surface>,
    commands: VecDeque<Command>,
    generation: u64,
    session: u64,
    connected: bool,
    replaced: u64,
}

pub struct Display {
    state: Mutex<State>,
    notify: Notify,
    session: watch::Sender<u64>,
    status: watch::Sender<Status>,
}

enum Job {
    Image {
        surface: Surface,
        uri: Arc<str>,
        panel: bool,
    },
    Command(Command),
}
impl Job {
    fn event(&self) -> serde_json::Value {
        match self {
            Self::Image {
                surface,
                uri,
                panel,
            } => {
                if *panel {
                    serde_json::json!({"event":"setFeedback", "context":surface.id, "payload":{"panel":uri.as_ref()}})
                } else {
                    serde_json::json!({"event":"setImage", "context":surface.id, "payload":{"image":uri.as_ref(), "state":0}})
                }
            }
            Self::Command(command) => command.event.clone(),
        }
    }
    fn context(&self) -> &str {
        match self {
            Self::Image { surface, .. } => &surface.id,
            Self::Command(c) => &c.surface.id,
        }
    }
}

#[async_trait::async_trait]
trait Transport: Send + Sync {
    async fn send(&self, event: serde_json::Value) -> Result<(), String>;
}
struct OpenAction;
#[async_trait::async_trait]
impl Transport for OpenAction {
    async fn send(&self, event: serde_json::Value) -> Result<(), String> {
        openaction::send_arbitrary_json(event)
            .await
            .map_err(|e| e.to_string())
    }
}

impl Display {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(State::default()),
            notify: Notify::new(),
            session: watch::channel(0).0,
            status: watch::channel(Status::Offline).0,
        })
    }
    pub fn subscribe(&self) -> watch::Receiver<Status> {
        self.status.subscribe()
    }
    pub fn start(self: &Arc<Self>) {
        let display = self.clone();
        tokio::spawn(async move { display.run(&OpenAction).await });
    }
    pub fn register(&self, id: String, panel: bool) -> Surface {
        let mut state = self.state.lock().unwrap();
        state.generation += 1;
        let surface = Surface {
            id: id.clone(),
            generation: state.generation,
        };
        // Remove old dirty tokens rather than retaining a queue proportional to appearances.
        state.dirty.retain(|s| s.id != id);
        state.commands.retain(|c| c.surface.id != id);
        state.entries.insert(
            id,
            Entry {
                surface: surface.clone(),
                panel,
                desired: None,
                last: None,
                due: Instant::now(),
                queued: false,
                sending: false,
            },
        );
        surface
    }
    pub fn remove(&self, surface: &Surface) {
        let mut state = self.state.lock().unwrap();
        if state
            .entries
            .get(&surface.id)
            .is_some_and(|e| e.surface == *surface)
        {
            state.entries.remove(&surface.id);
            state.dirty.retain(|s| s != surface);
        }
        state.commands.retain(|c| c.surface != *surface);
    }
    pub fn publish(&self, surface: &Surface, uri: String) {
        let mut state = self.state.lock().unwrap();
        let Some(entry) = state
            .entries
            .get_mut(&surface.id)
            .filter(|e| e.surface == *surface)
        else {
            return;
        };
        if entry.desired.as_deref() == Some(&uri)
            || (!entry.sending && entry.desired.is_none() && entry.last.as_deref() == Some(&uri))
        {
            return;
        }
        let replaced = entry.desired.replace(Arc::from(uri)).is_some();
        if !entry.queued {
            entry.queued = true;
            state.dirty.push_back(surface.clone());
        }
        state.replaced += u64::from(replaced);
        drop(state);
        self.notify.notify_one();
    }
    /// Only called once the SDK has installed its outbound manager, or after its reader exits.
    pub fn connected(&self, connected: bool) {
        let mut state = self.state.lock().unwrap();
        state.session += 1;
        state.connected = connected;
        eprintln!(
            "display connection {}",
            if connected { "ready" } else { "offline" }
        );
        state.commands.clear();
        for entry in state.entries.values_mut() {
            entry.last = None;
            entry.due = Instant::now();
            entry.sending = false;
        }
        self.session.send_replace(state.session);
        self.status.send_replace(if connected {
            Status::Ready
        } else {
            Status::Offline
        });
        drop(state);
        self.notify.notify_one();
    }
    pub fn redraw(&self) {
        for entry in self.state.lock().unwrap().entries.values_mut() {
            entry.last = None;
        }
        self.notify.notify_one();
    }
    pub fn command(
        &self,
        surface: &Surface,
        event: serde_json::Value,
    ) -> Result<oneshot::Receiver<Result<(), String>>, String> {
        let mut state = self.state.lock().unwrap();
        if !state.connected || *self.status.borrow() == Status::Stalled {
            return Err("host display connection is unavailable or stalled".into());
        }
        if !state
            .entries
            .get(&surface.id)
            .is_some_and(|e| e.surface == *surface)
        {
            return Err("surface is no longer visible".into());
        }
        if state.commands.len() >= COMMAND_LIMIT {
            return Err("host command queue is full".into());
        }
        let (reply, receiver) = oneshot::channel();
        state.commands.push_back(Command {
            surface: surface.clone(),
            event,
            expires: Instant::now() + DEADLINE,
            reply: Some(reply),
        });
        drop(state);
        self.notify.notify_one();
        Ok(receiver)
    }
    fn next(&self) -> Option<(u64, Job)> {
        let mut state = self.state.lock().unwrap();
        if !state.connected {
            return None;
        }
        while let Some(mut command) = state.commands.pop_front() {
            if command.expires <= Instant::now() {
                let _ = command
                    .reply
                    .take()
                    .unwrap()
                    .send(Err("host command expired before sending".into()));
                continue;
            }
            return Some((state.session, Job::Command(command)));
        }
        for _ in 0..state.dirty.len() {
            let surface = state.dirty.pop_front().unwrap();
            let Some(entry) = state
                .entries
                .get_mut(&surface.id)
                .filter(|e| e.surface == surface)
            else {
                continue;
            };
            if entry.due > Instant::now() {
                state.dirty.push_back(surface);
                continue;
            }
            entry.queued = false;
            let Some(uri) = entry.desired.take() else {
                continue;
            };
            if entry.last.as_ref() == Some(&uri) {
                continue;
            }
            let panel = entry.panel;
            entry.sending = true;
            return Some((
                state.session,
                Job::Image {
                    surface,
                    uri,
                    panel,
                },
            ));
        }
        None
    }

    fn expire_commands(&self) {
        let mut state = self.state.lock().unwrap();
        let now = Instant::now();
        while state.commands.front().is_some_and(|c| c.expires <= now) {
            let mut command = state.commands.pop_front().unwrap();
            eprintln!("host command expired context={}", command.surface.id);
            let _ = command
                .reply
                .take()
                .unwrap()
                .send(Err("host command expired before sending".into()));
        }
    }
    fn complete(&self, session: u64, job: Job, result: Result<(), String>) {
        let mut state = self.state.lock().unwrap();
        if session != state.session {
            return;
        }
        match job {
            Job::Image { surface, uri, .. } => {
                if let Some(entry) = state
                    .entries
                    .get_mut(&surface.id)
                    .filter(|e| e.surface == surface)
                {
                    entry.sending = false;
                    entry.due = Instant::now() + CADENCE;
                    if result.is_ok() {
                        entry.last = Some(uri);
                    } else if entry.desired.is_none() {
                        entry.desired = Some(uri);
                    }
                    if entry.desired.is_some() && !entry.queued {
                        entry.queued = true;
                        state.dirty.push_back(surface);
                    }
                }
            }
            Job::Command(mut command) => {
                if let Some(reply) = command.reply.take() {
                    let _ = reply.send(result.clone());
                }
            }
        }
        if let Err(error) = result {
            if error == "host command expired before sending"
                || error == "host command surface changed before sending"
            {
                self.status.send_replace(Status::Ready);
                return;
            }
            eprintln!("display connection failed: {error}");
            state.connected = false;
            state.commands.clear();
            self.status.send_replace(Status::Offline);
        } else {
            self.status.send_replace(Status::Ready);
        }
    }
    fn transition(&self, session: u64, status: Status) -> bool {
        let state = self.state.lock().unwrap();
        if state.session != session || !state.connected {
            return false;
        }
        self.status.send_replace(status);
        true
    }
    async fn run(&self, transport: &impl Transport) {
        let mut session_changed = self.session.subscribe();
        loop {
            // Register before inspecting work: publication cannot be lost between inspection and wait.
            let notified = self.notify.notified();
            let Some((session, mut job)) = self.next() else {
                tokio::select! { _ = notified => {}, _ = sleep(CADENCE) => {} }
                continue;
            };
            session_changed.borrow_and_update();
            if *session_changed.borrow() != session {
                continue;
            }
            if !self.transition(session, Status::Sending) {
                continue;
            }
            let eligibility = match &job {
                Job::Command(command) => Some((command.surface.clone(), command.expires)),
                _ => None,
            };
            let mut started = false;
            let mut sending = transport.send(job.event());
            // OpenAction resolves its global manager when polled. Gate every poll
            // against session end; never let old commands acquire a new manager.
            let send = std::future::poll_fn(|cx| {
                let state = self.state.lock().unwrap();
                if state.session != session || !state.connected {
                    return std::task::Poll::Ready(None);
                }
                if !started && let Some((surface, expires)) = &eligibility {
                    if *expires <= Instant::now() {
                        return std::task::Poll::Ready(Some(Err(
                            "host command expired before sending".into(),
                        )));
                    }
                    if !state
                        .entries
                        .get(&surface.id)
                        .is_some_and(|e| e.surface == *surface)
                    {
                        return std::task::Poll::Ready(Some(Err(
                            "host command surface changed before sending".into(),
                        )));
                    }
                }
                started = true;
                sending.as_mut().poll(cx).map(Some)
            });
            tokio::pin!(send);
            let result = tokio::select! {
                biased;
                _ = session_changed.changed() => None,
                result = &mut send => result,
                _ = sleep(DEADLINE) => {
                    if !self.transition(session, Status::Stalled) { continue }
                    eprintln!("display stalled context={} after 2s; retaining in-flight send", job.context());
                    if let Job::Command(command) = &mut job
                        && let Some(reply) = command.reply.take() {
                            let _ = reply.send(Err("host command stalled; delivery is uncertain and will not be replayed".into()));
                    }
                    // Keep polling the same future. Cancellation is only allowed when the old session ends.
                    loop {
                        self.expire_commands();
                        tokio::select! {
                            biased;
                            _ = session_changed.changed() => break None,
                            result = &mut send => {
                                eprintln!("display recovered context={}", job.context());
                                break result
                            },
                            _ = sleep(CADENCE) => {},
                        }
                    }
                }
            };
            if let Some(result) = result {
                self.complete(session, job, result);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Blocked {
        events: Mutex<Vec<serde_json::Value>>,
        release: Notify,
    }
    #[async_trait::async_trait]
    impl Transport for Blocked {
        async fn send(&self, event: serde_json::Value) -> Result<(), String> {
            self.events.lock().unwrap().push(event);
            self.release.notified().await;
            Ok(())
        }
    }
    #[tokio::test(start_paused = true)]
    async fn stalled_send_keeps_latest_and_does_not_block_lifecycle_or_commands() {
        let display = Display::new();
        let transport = Arc::new(Blocked {
            events: Mutex::new(Vec::new()),
            release: Notify::new(),
        });
        let d = display.clone();
        let t = transport.clone();
        let worker = tokio::spawn(async move { d.run(t.as_ref()).await });
        display.connected(true);
        let a = display.register("a".into(), true);
        let b = display.register("b".into(), false);
        display.publish(&a, "A".into());
        tokio::task::yield_now().await;
        let queued = display
            .command(&a, serde_json::json!({"event":"switchProfile"}))
            .unwrap();
        for value in ["B", "C", "D"] {
            display.publish(&a, value.into());
        }
        display.publish(&b, "tile".into());
        tokio::time::advance(DEADLINE).await;
        tokio::task::yield_now().await;
        assert_eq!(*display.subscribe().borrow(), Status::Stalled);
        assert!(display.command(&a, serde_json::json!({})).is_err());
        assert_eq!(display.state.lock().unwrap().replaced, 2);
        display.remove(&b);
        transport.release.notify_one();
        tokio::task::yield_now().await;
        assert!(queued.await.unwrap().unwrap_err().contains("expired"));
        tokio::time::advance(CADENCE).await;
        tokio::task::yield_now().await;
        let events = transport.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["payload"]["panel"], "D");
        worker.abort();
    }
    #[tokio::test]
    async fn fairness_deduplication_and_obsolete_completions() {
        let display = Display::new();
        display.connected(true);
        let a = display.register("a".into(), true);
        let b = display.register("b".into(), true);
        display.publish(&a, "one".into());
        display.publish(&b, "two".into());
        let (session, old) = display.next().unwrap();
        display.publish(&a, "three".into());
        assert_eq!(display.next().unwrap().1.context(), "b");
        let new = display.register("a".into(), true);
        display.publish(&new, "one".into());
        display.complete(session, old, Ok(()));
        assert_eq!(display.next().unwrap().1.context(), "a");
        display.publish(&a, "obsolete".into());
        assert!(display.next().is_none());
        assert!(display.state.lock().unwrap().dirty.is_empty());
    }

    #[tokio::test(start_paused = true)]
    async fn reverting_to_last_image_during_send_is_not_lost_and_commands_are_bounded() {
        let display = Display::new();
        display.connected(true);
        let a = display.register("a".into(), false);
        display.publish(&a, "A".into());
        let (session, job) = display.next().unwrap();
        display.complete(session, job, Ok(()));
        tokio::time::advance(CADENCE).await;
        display.publish(&a, "B".into());
        let (session, job) = display.next().unwrap();
        display.publish(&a, "A".into());
        display.complete(session, job, Ok(()));
        tokio::time::advance(CADENCE).await;
        assert_eq!(display.next().unwrap().1.event()["payload"]["image"], "A");
        let mut replies = Vec::new();
        for _ in 0..COMMAND_LIMIT {
            replies.push(display.command(&a, serde_json::json!({})).unwrap());
        }
        assert!(display.command(&a, serde_json::json!({})).is_err());
        display.connected(false);
        for reply in replies {
            assert!(reply.await.is_err());
        }
        assert!(display.next().is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn disconnect_cancels_old_send_and_reconnect_sends_current_state_without_commands() {
        let display = Display::new();
        let transport = Arc::new(Blocked {
            events: Mutex::new(Vec::new()),
            release: Notify::new(),
        });
        let d = display.clone();
        let t = transport.clone();
        let worker = tokio::spawn(async move { d.run(t.as_ref()).await });
        let a = display.register("a".into(), true);
        display.connected(true);
        display.publish(&a, "old".into());
        tokio::task::yield_now().await;
        display.publish(&a, "new".into());
        let reply = display
            .command(&a, serde_json::json!({"event":"switchProfile"}))
            .unwrap();
        display.connected(false);
        tokio::task::yield_now().await;
        assert!(reply.await.is_err());
        display.connected(true);
        tokio::task::yield_now().await;
        let events = transport.events.lock().unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[1]["payload"]["panel"], "new");
        worker.abort();
    }

    #[tokio::test(start_paused = true)]
    async fn in_flight_command_reports_uncertain_delivery_without_retry() {
        let display = Display::new();
        let transport = Arc::new(Blocked {
            events: Mutex::new(Vec::new()),
            release: Notify::new(),
        });
        let d = display.clone();
        let t = transport.clone();
        let worker = tokio::spawn(async move { d.run(t.as_ref()).await });
        display.connected(true);
        let a = display.register("a".into(), false);
        let reply = display
            .command(&a, serde_json::json!({"event":"switchProfile"}))
            .unwrap();
        tokio::task::yield_now().await;
        tokio::time::advance(DEADLINE).await;
        assert!(reply.await.unwrap().unwrap_err().contains("uncertain"));
        transport.release.notify_one();
        tokio::task::yield_now().await;
        assert_eq!(transport.events.lock().unwrap().len(), 1);
        worker.abort();
    }

    struct Failed;
    #[async_trait::async_trait]
    impl Transport for Failed {
        async fn send(&self, _: serde_json::Value) -> Result<(), String> {
            Err("closed".into())
        }
    }
    #[tokio::test]
    async fn transport_error_fails_command_and_stops_delivery_until_new_session() {
        let display = Display::new();
        display.connected(true);
        let a = display.register("a".into(), false);
        let reply = display.command(&a, serde_json::json!({})).unwrap();
        let d = display.clone();
        let worker = tokio::spawn(async move { d.run(&Failed).await });
        assert_eq!(reply.await.unwrap().unwrap_err(), "closed");
        assert_eq!(*display.subscribe().borrow(), Status::Offline);
        assert!(display.command(&a, serde_json::json!({})).is_err());
        worker.abort();
    }
}
