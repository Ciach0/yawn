use crate::event::{BuildEvent, Event, Level};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct Running {
    pub id: u32,
    pub target: String,
    pub started: Duration,
    pub done: u32,
    pub total: u32,
    pub label: String,
    /// Done and total of each parallel cargo build, by name (`client`, `server`)
    pub lanes: Vec<(String, u32, u32)>,
}

impl Running {
    pub fn lane(&self, name: &str) -> Option<(u32, u32)> {
        self.lanes.iter().find(|(n, ..)| n == name).map(|(_, d, t)| (*d, *t))
    }
}

#[derive(Clone, Debug)]
pub struct Finished {
    pub target: String,
    pub ok: bool,
    pub duration: Duration,
    pub warnings: u32,
    pub errors: u32,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Building,
    Ok,
    Failed,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Summary {
    pub errors: u64,
    pub warnings: u64,
    pub builds_ok: u32,
    pub builds_failed: u32,
}

/// What a session has done so far: the running build, the last finished one, totals and the named
/// status values. The log lines themselves are not kept.
#[derive(Default)]
pub struct State {
    now: Duration,
    running: Option<Running>,
    last: Option<Finished>,
    summary: Summary,
    ended: bool,
    chips: Vec<(String, String)>,
}

impl State {
    pub fn new() -> State {
        State::default()
    }

    pub fn apply(&mut self, event: &Event) {
        self.now = self.now.max(event.at());
        match event {
            Event::Log(l) => match l.level {
                Level::Error => self.summary.errors += 1,
                Level::Warn => self.summary.warnings += 1,
                _ => {}
            },
            Event::Build(b) => self.apply_build(b),
            Event::Info { key, value, .. } => match self.chips.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1.clone_from(value),
                None => self.chips.push((key.clone(), value.clone())),
            },
        }
    }

    /// Named status values in the order they first appeared
    pub fn chips(&self) -> &[(String, String)] {
        &self.chips
    }

    pub fn chip(&self, key: &str) -> Option<&str> {
        self.chips.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    fn apply_build(&mut self, b: &BuildEvent) {
        match b {
            BuildEvent::Started { at, id, target } => {
                self.running = Some(Running {
                    id: *id,
                    target: target.clone(),
                    started: *at,
                    done: 0,
                    total: 0,
                    label: String::new(),
                    lanes: Vec::new(),
                });
            }
            BuildEvent::Lane { id, lane, done, total, .. } => {
                if let Some(r) = self.running.as_mut().filter(|r| r.id == *id) {
                    match r.lanes.iter_mut().find(|(n, ..)| n == lane) {
                        Some(slot) => (slot.1, slot.2) = (*done, *total),
                        None => r.lanes.push((lane.clone(), *done, *total)),
                    }
                }
            }
            BuildEvent::Progress { id, done, total, label, .. } => {
                if let Some(r) = self.running.as_mut().filter(|r| r.id == *id) {
                    r.done = *done;
                    r.total = *total;
                    r.label.clone_from(label);
                }
            }
            BuildEvent::Finished { id, target, ok, duration, warnings, errors, .. } => {
                if self.running.as_ref().is_some_and(|r| r.id == *id) {
                    self.running = None;
                }
                if *ok {
                    self.summary.builds_ok += 1;
                } else {
                    self.summary.builds_failed += 1;
                }
                self.last = Some(Finished {
                    target: target.clone(),
                    ok: *ok,
                    duration: *duration,
                    warnings: *warnings,
                    errors: *errors,
                });
            }
        }
    }

    pub fn set_now(&mut self, now: Duration) {
        self.now = self.now.max(now);
    }

    pub fn mark_ended(&mut self) {
        self.ended = true;
    }

    pub fn running(&self) -> Option<&Running> {
        self.running.as_ref()
    }

    pub fn last_build(&self) -> Option<&Finished> {
        self.last.as_ref()
    }

    pub fn now(&self) -> Duration {
        self.now
    }

    pub fn ended(&self) -> bool {
        self.ended
    }

    pub fn summary(&self) -> &Summary {
        &self.summary
    }

    pub fn status(&self) -> Status {
        if self.running.is_some() {
            Status::Building
        } else {
            match &self.last {
                Some(f) if f.ok => Status::Ok,
                Some(_) => Status::Failed,
                None => Status::Idle,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::LogEvent;

    fn started(id: u32) -> Event {
        Event::Build(BuildEvent::Started { at: Duration::from_secs(1), id, target: "app".into() })
    }

    #[test]
    fn counts_errors_and_warnings() {
        let mut s = State::new();
        for level in [Level::Error, Level::Warn, Level::Info, Level::Error] {
            s.apply(&Event::Log(LogEvent::new(Duration::ZERO, level, "t", "x")));
        }
        assert_eq!((s.summary().errors, s.summary().warnings), (2, 1));
    }

    #[test]
    fn build_lifecycle() {
        let mut s = State::new();
        assert_eq!(s.status(), Status::Idle);
        s.apply(&started(1));
        assert_eq!(s.status(), Status::Building);
        s.apply(&Event::Build(BuildEvent::Progress {
            at: Duration::from_secs(2),
            id: 1,
            done: 2,
            total: 5,
            label: "parse".into(),
        }));
        assert_eq!(s.running().unwrap().done, 2);
        s.apply(&Event::Build(BuildEvent::Finished {
            at: Duration::from_secs(3),
            id: 1,
            target: "app".into(),
            ok: false,
            duration: Duration::from_secs(2),
            warnings: 1,
            errors: 1,
        }));
        assert_eq!(s.status(), Status::Failed);
        assert_eq!(s.summary().builds_failed, 1);
        assert!(s.running().is_none());
    }

    #[test]
    fn info_chips_update_in_place() {
        let mut s = State::new();
        let info =
            |k: &str, v: &str| Event::Info { at: Duration::ZERO, key: k.into(), value: v.into() };
        s.apply(&info("server", "starting"));
        s.apply(&info("client", "ok"));
        s.apply(&info("server", "http://localhost:3000"));
        assert_eq!(
            s.chips(),
            [("server".into(), "http://localhost:3000".into()), ("client".into(), "ok".into())]
        );
        assert_eq!(s.chip("server"), Some("http://localhost:3000"));
    }

    #[test]
    fn lanes_are_tracked_by_name_for_the_current_build() {
        let mut s = State::new();
        s.apply(&started(1));
        let lane = |id, name: &str, done| {
            Event::Build(BuildEvent::Lane {
                at: Duration::ZERO,
                id,
                lane: name.into(),
                done,
                total: 10,
            })
        };
        s.apply(&lane(1, "client", 3));
        s.apply(&lane(1, "server", 1));
        s.apply(&lane(1, "client", 4));
        s.apply(&lane(9, "client", 9));
        let r = s.running().unwrap();
        assert_eq!(
            (r.lane("client"), r.lane("server"), r.lane("x")),
            (Some((4, 10)), Some((1, 10)), None)
        );
    }

    #[test]
    fn stale_progress_is_ignored() {
        let mut s = State::new();
        s.apply(&started(2));
        s.apply(&Event::Build(BuildEvent::Progress {
            at: Duration::ZERO,
            id: 1,
            done: 9,
            total: 9,
            label: "old".into(),
        }));
        assert_eq!(s.running().unwrap().done, 0);
    }
}
