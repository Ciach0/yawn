use crate::env::{Caps, EnvInfo, Output};
use crate::error::CliError;
use crate::event::Level;
use crate::output::{Format, LineWriter, summary_line};
use crate::pipeline::{EventSource, Msg, Pipeline};
use crate::state::{State, Summary};
use crate::tui::app::SessionInfo;
use std::io::{self, Write};

pub struct Session {
    pub title: String,
    pub source: Box<dyn EventSource>,
    /// Minimum level shown initially (TUI) or printed (plain/JSON)
    pub level: Level,
}

pub struct Presentation {
    pub output: Output,
    pub color: bool,
    pub caps: Caps,
}

pub fn run(session: Session, p: &Presentation) -> Result<Summary, CliError> {
    let env = EnvInfo::collect(&p.caps, p.color);
    let mut pipe = Pipeline::spawn(session.source);
    let result = match p.output {
        Output::Tui => {
            let info = SessionInfo { title: session.title, env };
            crate::tui::run(&pipe, info, session.level)
        }
        Output::Plain | Output::Json => {
            let format = if p.output == Output::Json {
                Format::Json
            } else {
                Format::Plain { color: p.color }
            };
            let stdout = io::stdout().lock();
            let out = io::BufWriter::new(stdout);
            crate::pipeline::spawn_signals(pipe.tx.clone())?;
            run_lines(&pipe, out, format, session.level, &session.title, &env)
                .map_err(CliError::from)
        }
    };
    // Always stop the source, also after an error: this is where child processes are terminated.
    pipe.finish();
    result
}

/// Streams events as lines until the source ends or the reader closes the pipe
pub fn run_lines<W: Write>(
    pipe: &Pipeline,
    out: W,
    format: Format,
    level: Level,
    title: &str,
    env: &EnvInfo,
) -> io::Result<Summary> {
    let mut w = LineWriter::new(out, format, level);
    let banner = match format {
        Format::Plain { .. } => format!("yawn {} · {title}", env.version),
        Format::Json => format!(
            "{{\"kind\":\"session\",\"tool\":\"yawn\",\"version\":\"{}\",\"title\":\"{}\"}}",
            env.version,
            title.replace('\\', "\\\\").replace('"', "\\\"")
        ),
    };
    w.write_raw(&banner)?;
    let mut totals = State::new();
    loop {
        // Flush whenever the queue runs dry, so a live session streams line by line while a flood
        // still goes out in large buffered writes.
        let msg = match pipe.rx.try_recv() {
            Ok(m) => Ok(m),
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                w.flush()?;
                pipe.rx.recv()
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => break,
        };
        match msg {
            Ok(Msg::Event(e)) => {
                totals.apply(&e);
                if !w.write_event(&e)? {
                    break;
                }
            }
            Ok(Msg::SourceEnded) | Ok(Msg::Signal) | Err(_) => break,
            Ok(Msg::Input(_)) => {}
        }
    }
    let summary = totals.summary().clone();
    let elapsed = pipe.start.elapsed();
    match format {
        Format::Plain { .. } => {
            w.write_raw(&summary_line(&summary, elapsed))?;
        }
        Format::Json => {
            w.write_raw(&format!(
                "{{\"kind\":\"summary\",\"elapsed_ms\":{},\"errors\":{},\"warnings\":{},\"builds_ok\":{},\"builds_failed\":{}}}",
                elapsed.as_millis(),
                summary.errors,
                summary.warnings,
                summary.builds_ok,
                summary.builds_failed
            ))?;
        }
    }
    w.flush()?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{BuildEvent, Event};
    use crate::pipeline::EventSink;
    use std::time::Duration;

    struct Script;

    impl EventSource for Script {
        fn run(self: Box<Self>, sink: EventSink) {
            let at = Duration::ZERO;
            for id in 1..=2 {
                let failed = id == 2;
                sink.log(Level::Info, "test:watch", "change detected");
                sink.build(|at| BuildEvent::Started { at, id, target: "app".into() });
                if failed {
                    sink.log(Level::Error, "test:build", "type mismatch");
                }
                sink.send(Event::Build(BuildEvent::Finished {
                    at,
                    id,
                    target: "app".into(),
                    ok: !failed,
                    duration: Duration::from_millis(5),
                    warnings: 0,
                    errors: failed as u32,
                }));
            }
        }
    }

    fn env() -> EnvInfo {
        EnvInfo {
            version: "9.9.9",
            cwd: "~".into(),
            cargo_project: None,
            term: "t".into(),
            color: false,
        }
    }

    fn script() -> Pipeline {
        Pipeline::spawn(Box::new(Script))
    }

    #[test]
    fn plain_run_prints_banner_events_and_summary() {
        let mut buf = Vec::new();
        let s = run_lines(
            &script(),
            &mut buf,
            Format::Plain { color: false },
            Level::Info,
            "test",
            &env(),
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(out.lines().next().unwrap().contains("yawn 9.9.9"));
        assert!(out.contains("test:watch"));
        assert!(out.lines().last().unwrap().starts_with("done in"));
        assert!(!out.contains('\x1b'));
        assert_eq!(s.builds_ok + s.builds_failed, 2);
    }

    #[test]
    fn json_run_emits_valid_looking_lines() {
        let mut buf = Vec::new();
        run_lines(&script(), &mut buf, Format::Json, Level::Info, "test", &env()).unwrap();
        let out = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = out.lines().collect();
        assert!(lines[0].starts_with("{\"kind\":\"session\""));
        assert!(lines.iter().all(|l| l.starts_with('{') && l.ends_with('}')));
        assert!(lines.last().unwrap().contains("\"kind\":\"summary\""));
        assert!(lines.iter().any(|l| l.contains("\"kind\":\"build_finished\"")));
    }

    #[test]
    fn level_filter_applies_to_plain_output_but_not_to_totals() {
        let mut buf = Vec::new();
        let s = run_lines(
            &script(),
            &mut buf,
            Format::Plain { color: false },
            Level::Error,
            "test",
            &env(),
        )
        .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(!out.contains("test:watch"));
        assert!(out.contains("ERROR"));
        assert_eq!(s.errors, 1);
    }
}
