use crate::event::{BuildEvent, Event, Level, LogEvent};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

pub const QUEUE_CAPACITY: usize = 8192;

pub enum Msg {
    Event(Event),
    /// The producer finished; no more events will come
    SourceEnded,
    Input(ratatui::crossterm::event::Event),
    /// SIGINT, SIGTERM or SIGHUP
    Signal,
}

type Hook = Box<dyn FnOnce() + Send>;

#[derive(Default)]
struct ShutdownInner {
    done: bool,
    hooks: Vec<Hook>,
}

/// Shared shutdown flag with hooks
#[derive(Clone, Default)]
pub struct Shutdown {
    inner: Arc<(Mutex<ShutdownInner>, Condvar)>,
}

impl Shutdown {
    /// Registers `hook` to run on shutdown (immediately if it already happened)
    pub fn on_shutdown(&self, hook: impl FnOnce() + Send + 'static) {
        let mut g = self.inner.0.lock().unwrap();
        if g.done {
            drop(g);
            hook();
        } else {
            g.hooks.push(Box::new(hook));
        }
    }

    pub fn trigger(&self) {
        let hooks = {
            let mut g = self.inner.0.lock().unwrap();
            if g.done {
                return;
            }
            g.done = true;
            std::mem::take(&mut g.hooks)
        };
        self.inner.1.notify_all();
        for h in hooks {
            h();
        }
    }

    pub fn is_triggered(&self) -> bool {
        self.inner.0.lock().unwrap().done
    }

    /// Sleeps up to `d`; returns `true` if shutdown happened (before or during the wait)
    pub fn wait(&self, d: Duration) -> bool {
        let g = self.inner.0.lock().unwrap();
        let (g, _) = self.inner.1.wait_timeout_while(g, d, |s| !s.done).unwrap();
        g.done
    }
}

/// "Reload" asked for by the person at the keyboard (`r`): the source registers what it does, the
/// presenter triggers it
#[derive(Clone, Default)]
pub struct ReloadHandle {
    hook: Arc<Mutex<Option<Box<dyn Fn() + Send>>>>,
}

impl ReloadHandle {
    pub fn on_reload(&self, hook: impl Fn() + Send + 'static) {
        *self.hook.lock().unwrap() = Some(Box::new(hook));
    }

    /// Runs the hook; `false` when the source has none
    pub fn trigger(&self) -> bool {
        match &*self.hook.lock().unwrap() {
            Some(h) => {
                h();
                true
            }
            None => false,
        }
    }
}

/// Producer side
pub struct EventSink {
    tx: SyncSender<Msg>,
    start: Instant,
    shutdown: Shutdown,
    reload: ReloadHandle,
}

impl EventSink {
    pub fn reload(&self) -> &ReloadHandle {
        &self.reload
    }

    pub fn elapsed(&self) -> Duration {
        self.start.elapsed()
    }

    pub fn shutdown(&self) -> &Shutdown {
        &self.shutdown
    }

    /// Interruptible sleep; `false` means the session is shutting down and the source should stop
    pub fn sleep(&self, d: Duration) -> bool {
        !self.shutdown.wait(d)
    }

    /// Returns `false` once the consumer is gone; producers should stop then
    pub fn send(&self, event: Event) -> bool {
        self.tx.send(Msg::Event(event)).is_ok()
    }

    pub fn log(&self, level: Level, source: &'static str, message: impl AsRef<str>) -> bool {
        self.send(Event::Log(LogEvent::new(self.elapsed(), level, source, message)))
    }

    pub fn info(&self, key: &str, value: impl AsRef<str>) -> bool {
        self.send(Event::Info {
            at: self.elapsed(),
            key: crate::event::clean(key),
            value: crate::event::clean(value.as_ref()),
        })
    }

    pub fn build(&self, make: impl FnOnce(Duration) -> BuildEvent) -> bool {
        self.send(Event::Build(make(self.elapsed())))
    }
}

impl Drop for EventSink {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::SourceEnded);
    }
}

/// Anything that produces events: the dev supervisor
pub trait EventSource: Send + 'static {
    fn run(self: Box<Self>, sink: EventSink);
}

pub struct Pipeline {
    pub rx: Receiver<Msg>,
    pub tx: SyncSender<Msg>,
    pub start: Instant,
    shutdown: Shutdown,
    pub reload: ReloadHandle,
    handle: Option<JoinHandle<()>>,
}

impl Pipeline {
    /// Creates the queue and starts `source` on a background thread
    pub fn spawn(source: Box<dyn EventSource>) -> Pipeline {
        let (tx, rx) = sync_channel(QUEUE_CAPACITY);
        let start = Instant::now();
        let shutdown = Shutdown::default();
        let reload = ReloadHandle::default();
        let sink =
            EventSink { tx: tx.clone(), start, shutdown: shutdown.clone(), reload: reload.clone() };
        let handle = std::thread::Builder::new()
            .name("yawn-source".into())
            .spawn(move || source.run(sink))
            .expect("spawn event source thread");
        Pipeline { rx, tx, start, shutdown, reload, handle: Some(handle) }
    }

    /// Stops the source: runs its shutdown hooks and waits (at most 5 s) for its thread, which is
    /// where child processes get terminated
    pub fn finish(&mut self) {
        self.shutdown.trigger();
        // Drain so a producer blocked on a full queue can observe the shutdown.
        let deadline = Instant::now() + Duration::from_secs(5);
        if let Some(h) = self.handle.take() {
            while !h.is_finished() && Instant::now() < deadline {
                while self.rx.try_recv().is_ok() {}
                std::thread::sleep(Duration::from_millis(5));
            }
            if h.is_finished() {
                let _ = h.join();
            }
        }
    }
}

/// SIGINT/SIGTERM/SIGHUP from outside (`kill`, closing the terminal, Ctrl-C when the terminal is
/// not in raw mode) become a `Msg::Signal`, so the presenter can shut down cleanly
#[cfg(unix)]
pub fn spawn_signals(tx: SyncSender<Msg>) -> std::io::Result<()> {
    use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
    let mut signals = signal_hook::iterator::Signals::new([SIGINT, SIGTERM, SIGHUP])?;
    std::thread::Builder::new().name("yawn-signals".into()).spawn(move || {
        for _ in signals.forever() {
            if tx.send(Msg::Signal).is_err() {
                break;
            }
        }
    })?;
    Ok(())
}

#[cfg(not(unix))]
pub fn spawn_signals(_tx: SyncSender<Msg>) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shutdown_runs_hooks_once_and_wakes_waiters() {
        let s = Shutdown::default();
        let n = Arc::new(Mutex::new(0));
        let n2 = n.clone();
        s.on_shutdown(move || *n2.lock().unwrap() += 1);
        let s2 = s.clone();
        let waiter = std::thread::spawn(move || s2.wait(Duration::from_secs(10)));
        std::thread::sleep(Duration::from_millis(30));
        let t = Instant::now();
        s.trigger();
        s.trigger();
        assert!(waiter.join().unwrap());
        assert!(t.elapsed() < Duration::from_secs(1), "waiter woke immediately");
        assert_eq!(*n.lock().unwrap(), 1);
        let n3 = n.clone();
        s.on_shutdown(move || *n3.lock().unwrap() += 10);
        assert_eq!(*n.lock().unwrap(), 11, "late hooks run immediately");
    }

    #[test]
    fn wait_times_out_without_shutdown() {
        assert!(!Shutdown::default().wait(Duration::from_millis(10)));
    }

    struct Sleeper;
    impl EventSource for Sleeper {
        fn run(self: Box<Self>, sink: EventSink) {
            while sink.sleep(Duration::from_secs(60)) {}
        }
    }

    #[test]
    fn finish_stops_a_sleeping_source_quickly() {
        let mut p = Pipeline::spawn(Box::new(Sleeper));
        let t = Instant::now();
        p.finish();
        assert!(t.elapsed() < Duration::from_secs(2));
    }
}
