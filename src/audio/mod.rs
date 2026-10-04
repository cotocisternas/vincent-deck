//! Persistent native PipeWire/WirePlumber audio control, isolated on one thread.
//! The UI receives latest-state notifications; no CLI tools or text parsing.
use anyhow::{Result, anyhow, ensure};
use glib::translate::ToGlibPtr;
use std::{sync::mpsc, time::Duration};
use tokio::sync::{oneshot, watch};

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Level {
    pub percent: u32,
    pub muted: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct State {
    pub output: Option<Level>,
    pub input: Option<Level>,
    pub output_device: String,
    pub input_device: String,
}

enum Operation {
    Adjust(Option<i64>),
    Cycle,
}

struct Request {
    source: bool,
    operation: Operation,
    reply: oneshot::Sender<Result<()>>,
}

pub struct Audio {
    commands: mpsc::SyncSender<Request>,
    context: glib::MainContext,
    state: watch::Receiver<State>,
}

impl Audio {
    pub fn new() -> Self {
        Self::start(None)
    }

    /// Connect to an explicit socket (e.g. an isolated test audio session).
    pub fn connect(remote: &str) -> Result<Self> {
        Ok(Self::start(Some(std::ffi::CString::new(remote)?)))
    }

    fn start(remote: Option<std::ffi::CString>) -> Self {
        let (commands, rx) = mpsc::sync_channel(32);
        let (tx, state) = watch::channel(State::default());
        let context = glib::MainContext::new();
        let thread_context = context.clone();
        std::thread::Builder::new()
            .name("vincent-audio".into())
            .spawn(move || {
                thread_context
                    .with_thread_default(|| run(&thread_context, remote.as_deref(), rx, tx))
                    .expect("audio thread owns its GLib context");
            })
            .expect("start native audio thread");
        Self {
            commands,
            context,
            state,
        }
    }

    pub fn subscribe(&self) -> watch::Receiver<State> {
        self.state.clone()
    }

    pub async fn adjust(&self, source: bool, ticks: Option<i64>) -> Result<()> {
        self.request(source, Operation::Adjust(ticks)).await
    }

    pub async fn cycle_device(&self, source: bool) -> Result<()> {
        self.request(source, Operation::Cycle).await
    }

    async fn request(&self, source: bool, operation: Operation) -> Result<()> {
        let (reply, result) = oneshot::channel();
        self.commands
            .try_send(Request {
                source,
                operation,
                reply,
            })
            .map_err(|error| anyhow!("audio command unavailable: {error}"))?;
        self.context.invoke(|| {});
        result.await.map_err(|_| anyhow!("audio thread stopped"))?
    }
}

impl Default for Audio {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Audio {
    fn drop(&mut self) {
        // Closing the command channel followed by wakeup lets the thread release
        // its native proxies even when the PipeWire connection is idle.
        let (replacement, _) = mpsc::sync_channel(1);
        self.commands = replacement;
        self.context.invoke(|| {});
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct NativeValue {
    id: u32,
    volume: f64,
    muted: i32,
    known: i32,
}

impl NativeValue {
    fn level(self) -> Option<Level> {
        (self.known != 0
            && self.volume.is_finite()
            && self.volume >= 0.0
            && self.volume <= u32::MAX as f64 / 100.0)
            .then(|| Level {
                percent: (self.volume * 100.0).round() as u32,
                muted: self.muted != 0,
            })
    }
}

unsafe extern "C" {
    fn vincent_audio_new(
        context: *mut glib::ffi::GMainContext,
        remote: *const std::ffi::c_char,
    ) -> *mut std::ffi::c_void;
    fn vincent_audio_failed(audio: *mut std::ffi::c_void) -> i32;
    fn vincent_audio_dirty(audio: *mut std::ffi::c_void) -> i32;
    fn vincent_audio_read(audio: *mut std::ffi::c_void, source: i32) -> NativeValue;
    fn vincent_audio_label(
        audio: *mut std::ffi::c_void,
        id: u32,
        label: *mut std::ffi::c_char,
        capacity: usize,
    );
    fn vincent_audio_cycle(audio: *mut std::ffi::c_void, source: i32) -> i32;
    fn vincent_audio_write(audio: *mut std::ffi::c_void, id: u32, volume: f64, mute: i32) -> i32;
    fn vincent_audio_free(audio: *mut std::ffi::c_void);
}

// Created, used, and dropped exclusively on the owning GLib thread.
struct Session(*mut std::ffi::c_void);
impl Session {
    fn new(context: &glib::MainContext, remote: Option<&std::ffi::CStr>) -> Self {
        Self(unsafe {
            vincent_audio_new(
                context.to_glib_none().0,
                remote.map_or(std::ptr::null(), |s| s.as_ptr()),
            )
        })
    }
    fn failed(&self) -> bool {
        unsafe { vincent_audio_failed(self.0) != 0 }
    }
    fn dirty(&self) -> bool {
        unsafe { vincent_audio_dirty(self.0) != 0 }
    }
    fn read(&self, source: bool) -> NativeValue {
        unsafe { vincent_audio_read(self.0, i32::from(source)) }
    }
    fn label(&self, value: NativeValue) -> String {
        let mut buffer = [0_u8; 256];
        unsafe {
            vincent_audio_label(self.0, value.id, buffer.as_mut_ptr().cast(), buffer.len());
        }
        let end = buffer.iter().position(|b| *b == 0).unwrap_or(buffer.len());
        String::from_utf8_lossy(&buffer[..end])
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect()
    }

    fn cycle(&self, source: bool) -> Result<()> {
        ensure!(
            unsafe { vincent_audio_cycle(self.0, i32::from(source)) } != 0,
            "audio device switch could not be confirmed"
        );
        Ok(())
    }
    fn adjust(&self, source: bool, ticks: Option<i64>) -> Result<()> {
        let value = self.read(source);
        let level = value
            .level()
            .ok_or_else(|| anyhow!("default audio device unavailable"))?;
        let (volume, mute) = match ticks {
            Some(ticks) => (adjusted_percent(level.percent, ticks) as f64 / 100.0, -1),
            None => (0.0, i32::from(!level.muted)),
        };
        ensure!(
            unsafe { vincent_audio_write(self.0, value.id, volume, mute) } != 0,
            "native audio adjustment failed"
        );
        Ok(())
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        unsafe { vincent_audio_free(self.0) };
    }
}

fn adjusted_percent(percent: u32, ticks: i64) -> u32 {
    (percent as i64).saturating_add(ticks).clamp(0, 100) as u32
}

fn publish(session: &Session, tx: &watch::Sender<State>) {
    let output = session.read(false);
    let input = session.read(true);
    let next = State {
        output: output.level(),
        input: input.level(),
        output_device: session.label(output),
        input_device: session.label(input),
    };
    tx.send_if_modified(|state| {
        if *state == next {
            return false;
        }
        *state = next;
        true
    });
}

fn run(
    context: &glib::MainContext,
    remote: Option<&std::ffi::CStr>,
    rx: mpsc::Receiver<Request>,
    tx: watch::Sender<State>,
) {
    loop {
        let session = Session::new(context, remote);
        while !session.failed() {
            loop {
                match rx.try_recv() {
                    Ok(request) => {
                        if request.reply.is_closed() {
                            continue;
                        }
                        let result = match request.operation {
                            Operation::Adjust(ticks) => session.adjust(request.source, ticks),
                            Operation::Cycle => session.cycle(request.source),
                        };
                        publish(&session, &tx);
                        let _ = request.reply.send(result);
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => return,
                }
            }
            if session.dirty() {
                publish(&session, &tx);
            }
            if !session.failed() {
                context.iteration(true);
            }
        }
        tx.send_if_modified(|state| {
            let changed = *state != State::default();
            *state = State::default();
            changed
        });
        drop(session);
        // Consume failed commands during downtime; never replay after reconnect.
        match rx.recv_timeout(Duration::from_millis(500)) {
            Ok(request) => {
                let _ = request
                    .reply
                    .send(Err(anyhow!("audio connection unavailable")));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_tick_and_above_cap_policy() {
        assert_eq!(adjusted_percent(120, -1), 100);
        assert_eq!(adjusted_percent(50, 5), 55);
        assert_eq!(adjusted_percent(50, i64::MIN), 0);
        assert_eq!(adjusted_percent(50, i64::MAX), 100);
        let value = NativeValue {
            volume: 1.2,
            known: 1,
            muted: 1,
            ..Default::default()
        };
        assert_eq!(
            value.level(),
            Some(Level {
                percent: 120,
                muted: true
            })
        );
        assert_eq!(NativeValue::default().level(), None);
        assert_eq!(
            NativeValue {
                volume: f64::NAN,
                ..value
            }
            .level(),
            None
        );
    }
}
