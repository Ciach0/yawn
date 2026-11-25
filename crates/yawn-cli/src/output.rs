use crate::event::{BuildEvent, Event, Level, LogEvent, fmt_count, fmt_duration};
use std::io::{self, Write};

const RESET: &str = "\x1b[0m";
const DIM: &str = "\x1b[2m";

fn level_color(l: Level) -> &'static str {
    match l {
        Level::Error => "\x1b[1;31m",
        Level::Warn => "\x1b[33m",
        Level::Info => "\x1b[36m",
        Level::Debug | Level::Trace => DIM,
    }
}

/// `   12.345s INFO  source  message  at file:1:2 [E0001]`
pub fn format_plain(l: &LogEvent, color: bool) -> String {
    let mut s = String::with_capacity(96 + l.message.len());
    let time = format!("{:>9.3}s", l.at.as_secs_f64());
    let tail = {
        let mut t = String::new();
        if let Some(loc) = &l.location {
            t.push_str(&format!("  at {loc}"));
        }
        if let Some(c) = &l.code {
            t.push_str(&format!(" [{c}]"));
        }
        t
    };
    if color {
        s.push_str(&format!(
            "{DIM}{time}{RESET} {}{}{RESET} {DIM}{:<10}{RESET} {}{}",
            level_color(l.level),
            l.level.label(),
            l.source,
            l.message,
            if tail.is_empty() { String::new() } else { format!("{DIM}{tail}{RESET}") }
        ));
    } else {
        s.push_str(&format!("{time} {} {:<10} {}{tail}", l.level.label(), l.source, l.message));
    }
    s
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// One compact JSON object per event, stable field names: `t` (seconds), `kind`, `level`, `source`,
/// `message`, optional `file`/`line`/`col`/`code`, and for build events `build` with `id`,
/// `target`, and progress/result fields
pub fn format_json(e: &Event) -> String {
    let l = e.to_log();
    let mut s = String::with_capacity(160);
    let kind = match e {
        Event::Log(_) => "log",
        Event::Info { .. } => "status",
        Event::Build(BuildEvent::Started { .. }) => "build_started",
        Event::Build(BuildEvent::Progress { .. }) => "build_progress",
        Event::Build(BuildEvent::Lane { .. }) => "build_lane",
        Event::Build(BuildEvent::Finished { .. }) => "build_finished",
    };
    s.push_str(&format!(
        "{{\"t\":{:.3},\"kind\":\"{kind}\",\"level\":\"{}\",\"source\":",
        l.at.as_secs_f64(),
        l.level
    ));
    json_str(&mut s, &l.source);
    s.push_str(",\"message\":");
    json_str(&mut s, &l.message);
    if let Some(loc) = &l.location {
        s.push_str(",\"file\":");
        json_str(&mut s, &loc.file);
        s.push_str(&format!(",\"line\":{},\"col\":{}", loc.line, loc.col));
    }
    if let Some(c) = &l.code {
        s.push_str(",\"code\":");
        json_str(&mut s, c);
    }
    if let Event::Info { key, value, .. } = e {
        s.push_str(",\"key\":");
        json_str(&mut s, key);
        s.push_str(",\"value\":");
        json_str(&mut s, value);
    }
    if let Event::Build(b) = e {
        match b {
            BuildEvent::Started { id, target, .. } => {
                s.push_str(&format!(",\"build\":{{\"id\":{id},\"target\":"));
                json_str(&mut s, target);
                s.push('}');
            }
            BuildEvent::Progress { id, done, total, label, .. } => {
                s.push_str(&format!(
                    ",\"build\":{{\"id\":{id},\"done\":{done},\"total\":{total},\"label\":"
                ));
                json_str(&mut s, label);
                s.push('}');
            }
            BuildEvent::Lane { id, lane, done, total, .. } => {
                s.push_str(&format!(",\"build\":{{\"id\":{id},\"lane\":"));
                json_str(&mut s, lane);
                s.push_str(&format!(",\"done\":{done},\"total\":{total}}}"));
            }
            BuildEvent::Finished { id, target, ok, duration, warnings, errors, .. } => {
                s.push_str(&format!(",\"build\":{{\"id\":{id},\"target\":"));
                json_str(&mut s, target);
                s.push_str(&format!(
                    ",\"ok\":{ok},\"duration_ms\":{},\"warnings\":{warnings},\"errors\":{errors}}}",
                    duration.as_millis()
                ));
            }
        }
    }
    s.push('}');
    s
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Plain { color: bool },
    Json,
}

/// Writes events one per line
pub struct LineWriter<W: Write> {
    out: W,
    format: Format,
    min: Level,
}

impl<W: Write> LineWriter<W> {
    pub fn new(out: W, format: Format, min: Level) -> Self {
        LineWriter { out, format, min }
    }

    pub fn write_event(&mut self, e: &Event) -> io::Result<bool> {
        let l = e.to_log();
        // Build progress is detail; everything else obeys the level filter.
        if l.level < self.min {
            return Ok(true);
        }
        let line = match self.format {
            Format::Plain { color } => format_plain(&l, color),
            Format::Json => format_json(e),
        };
        match writeln!(self.out, "{line}") {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn write_raw(&mut self, line: &str) -> io::Result<bool> {
        match writeln!(self.out, "{line}") {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(false),
            Err(e) => Err(e),
        }
    }

    pub fn flush(&mut self) -> io::Result<()> {
        match self.out.flush() {
            Err(e) if e.kind() == io::ErrorKind::BrokenPipe => Ok(()),
            r => r,
        }
    }
}

/// Final one-line summary of a plain run
pub fn summary_line(s: &crate::state::Summary, elapsed: std::time::Duration) -> String {
    format!(
        "done in {} · {} error{}, {} warning{} · builds: {} ok, {} failed",
        fmt_duration(elapsed),
        fmt_count(s.errors),
        if s.errors == 1 { "" } else { "s" },
        fmt_count(s.warnings),
        if s.warnings == 1 { "" } else { "s" },
        s.builds_ok,
        s.builds_failed
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::Location;
    use std::time::Duration;

    fn ev() -> LogEvent {
        LogEvent::new(Duration::from_millis(1234), Level::Error, "sim:build", "type mismatch")
            .at_location(Location::new("src/a.rsx", 12, 5))
            .with_code("E0308")
    }

    #[test]
    fn plain_line_without_color_has_no_escapes() {
        let s = format_plain(&ev(), false);
        assert_eq!(s, "    1.234s ERROR sim:build  type mismatch  at src/a.rsx:12:5 [E0308]");
        assert!(!s.contains('\x1b'));
    }

    #[test]
    fn plain_line_with_color_has_escapes() {
        assert!(format_plain(&ev(), true).contains("\x1b[1;31m"));
    }

    #[test]
    fn json_is_escaped_and_complete() {
        let mut l = ev();
        l.message = "quote \" backslash \\ tab\u{1}".into();
        let j = format_json(&Event::Log(l));
        assert!(j.starts_with("{\"t\":1.234,\"kind\":\"log\",\"level\":\"error\""));
        assert!(j.contains(r#""message":"quote \" backslash \\ tab\u0001""#));
        assert!(j.contains(r#""file":"src/a.rsx","line":12,"col":5"#));
        assert!(j.contains(r#""code":"E0308""#));
        assert!(j.ends_with('}'));
    }

    #[test]
    fn json_build_finished_has_result_fields() {
        let j = format_json(&Event::Build(BuildEvent::Finished {
            at: Duration::from_secs(2),
            id: 3,
            target: "app".into(),
            ok: true,
            duration: Duration::from_millis(1500),
            warnings: 2,
            errors: 0,
        }));
        assert!(j.contains(r#""kind":"build_finished""#));
        assert!(j.contains(r#""ok":true,"duration_ms":1500,"warnings":2,"errors":0"#));
    }

    #[test]
    fn writer_applies_level_filter_and_survives_broken_pipe() {
        let mut buf = Vec::new();
        let mut w = LineWriter::new(&mut buf, Format::Plain { color: false }, Level::Warn);
        w.write_event(&Event::Log(LogEvent::new(Duration::ZERO, Level::Info, "s", "hidden")))
            .unwrap();
        w.write_event(&Event::Log(LogEvent::new(Duration::ZERO, Level::Warn, "s", "shown")))
            .unwrap();
        let out = String::from_utf8(buf).unwrap();
        assert!(!out.contains("hidden") && out.contains("shown"));

        struct Broken;
        impl Write for Broken {
            fn write(&mut self, _: &[u8]) -> io::Result<usize> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
            fn flush(&mut self) -> io::Result<()> {
                Err(io::ErrorKind::BrokenPipe.into())
            }
        }
        let mut w = LineWriter::new(Broken, Format::Json, Level::Trace);
        assert!(
            !w.write_event(&Event::Log(LogEvent::new(Duration::ZERO, Level::Error, "s", "x")))
                .unwrap()
        );
        assert!(w.flush().is_ok());
    }
}
