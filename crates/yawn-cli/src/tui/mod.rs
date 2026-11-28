pub mod app;
pub mod theme;
pub mod view;

use crate::error::CliError;
use crate::event::Level;
use crate::pipeline::{Msg, Pipeline};
use crate::state::Summary;
use app::{Act, App, SessionInfo};
use ratatui::backend::CrosstermBackend;
use ratatui::crossterm::ExecutableCommand;
use ratatui::crossterm::cursor::{Hide, MoveTo, Show};
use ratatui::crossterm::event::{self, Event as TermEvent};
use ratatui::crossterm::terminal::{Clear, ClearType, disable_raw_mode, enable_raw_mode};
use ratatui::layout::Position;
use ratatui::widgets::Paragraph;
use ratatui::{Terminal, TerminalOptions, Viewport};
use std::io::{self, Stdout, Write, stdout};
use std::process::{Command, Stdio};
use std::sync::mpsc::RecvTimeoutError;
use std::time::Duration;
use theme::Theme;

type Term = Terminal<CrosstermBackend<Stdout>>;

const TICK: Duration = Duration::from_millis(250);
const BATCH: usize = 16_384;
const LINES_PER_INSERT: usize = 500;

/// The raw terminal and the inline panel, put back as they were when this is dropped
struct Tui {
    term: Term,
}

impl Tui {
    fn start(height: u16) -> io::Result<Tui> {
        enable_raw_mode()?;
        stdout()
            .execute(Clear(ClearType::All))?
            .execute(Clear(ClearType::Purge))?
            .execute(MoveTo(0, 0))?;
        let term = open(height).inspect_err(|_| {
            let _ = disable_raw_mode();
        })?;
        stdout().execute(Hide)?;
        Ok(Tui { term })
    }
}

impl Drop for Tui {
    fn drop(&mut self) {
        let area = self.term.get_frame().area();
        let _ = self.term.set_cursor_position(Position { x: 0, y: area.y + area.height });
        let _ = stdout().execute(Show);
        let _ = disable_raw_mode();
        println!();
    }
}

fn open(height: u16) -> io::Result<Term> {
    Terminal::with_options(
        CrosstermBackend::new(stdout()),
        TerminalOptions { viewport: Viewport::Inline(height) },
    )
}

pub fn run(pipe: &Pipeline, info: SessionInfo, level: Level) -> Result<Summary, CliError> {
    crate::pipeline::spawn_signals(pipe.tx.clone())?;
    let mut tui = Tui::start(view::SMALL)?;
    spawn_input(pipe);
    event_loop(pipe, &mut tui.term, info, level)
}

/// Applies one message
fn handle(app: &mut App, msg: Msg) -> Act {
    match msg {
        Msg::Event(e) => {
            app.on_event(e);
            Act::Redraw
        }
        Msg::SourceEnded => {
            app.state.mark_ended();
            Act::Redraw
        }
        Msg::Signal => {
            app.quit = true;
            Act::Redraw
        }
        Msg::Input(TermEvent::Key(k)) => app.on_key(k),
        Msg::Input(TermEvent::Resize(..)) => Act::Reflow,
        Msg::Input(_) => Act::Nothing,
    }
}

fn event_loop(
    pipe: &Pipeline,
    term: &mut Term,
    info: SessionInfo,
    level: Level,
) -> Result<Summary, CliError> {
    let theme = Theme { color: info.env.color };
    let mut app = App::new(info, level);
    app.reload = Some(pipe.reload.clone());
    let mut dirty = true;

    loop {
        print_logs(term, &mut app, &theme)?;
        if app.quit {
            break;
        }
        if dirty {
            term.draw(|f| view::draw(f, &app, &theme))?;
            dirty = false;
        }
        let first = if app.state.running().is_some() {
            match pipe.rx.recv_timeout(TICK) {
                Ok(m) => m,
                Err(RecvTimeoutError::Timeout) => {
                    app.state.set_now(pipe.start.elapsed());
                    dirty = true;
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        } else {
            match pipe.rx.recv() {
                Ok(m) => m,
                Err(_) => break,
            }
        };
        let mut reflow = false;
        let mut acts = vec![handle(&mut app, first)];
        for _ in 0..BATCH {
            match pipe.rx.try_recv() {
                Ok(m) => acts.push(handle(&mut app, m)),
                Err(_) => break,
            }
            if app.quit {
                break;
            }
        }
        app.state.set_now(pipe.start.elapsed());
        for act in acts {
            dirty |= act != Act::Nothing;
            match act {
                Act::Clear => {
                    app.history.clear();
                    app.pending.clear();
                    reflow = true;
                }
                Act::Reflow => reflow = true,
                Act::Resize => resize(term, &mut app)?,
                Act::Open(url) => {
                    if let Err(e) = open_url(&url) {
                        app.on_event(crate::event::Event::Log(crate::event::LogEvent::new(
                            app.state.now(),
                            Level::Error,
                            "yawn",
                            format!("cannot open {url}: {e}"),
                        )));
                    }
                }
                Act::Nothing | Act::Redraw => {}
            }
        }
        if reflow {
            reprint(term, &mut app)?;
            dirty = true;
        }
    }
    Ok(app.state.summary().clone())
}

/// Prints the waiting log lines above the panel, where the terminal keeps them in its scrollback
fn print_logs(term: &mut Term, app: &mut App, theme: &Theme) -> io::Result<()> {
    if app.pending.is_empty() {
        return Ok(());
    }
    let width = term.size()?.width as usize;
    let printed = std::mem::take(&mut app.pending);
    let lines: Vec<_> = printed.iter().flat_map(|e| view::log_lines(e, theme, width)).collect();
    app.history.extend(printed);
    if app.history.len() > app::HISTORY {
        let extra = app.history.len() - app::HISTORY;
        app.history.drain(..extra);
    }
    for chunk in lines.chunks(LINES_PER_INSERT) {
        let height = chunk.len() as u16;
        let paragraph = Paragraph::new(chunk.to_vec());
        term.insert_before(height, |buf| {
            ratatui::widgets::Widget::render(paragraph, buf.area, buf)
        })?;
    }
    Ok(())
}

/// Wipes the screen and the scrollback, opens the panel at the top and queues what was printed
/// before to be printed again (at the width the terminal has now), so that a resize or a clear
/// always leaves a consistent screen
fn reprint(term: &mut Term, app: &mut App) -> io::Result<()> {
    if app.more && app.rustc.is_none() {
        app.rustc = Some(rustc_version());
    }
    let mut out = stdout();
    out.execute(Clear(ClearType::All))?.execute(Clear(ClearType::Purge))?.execute(MoveTo(0, 0))?;
    *term = open(app.height())?;
    let mut again = std::mem::take(&mut app.history);
    again.append(&mut app.pending);
    app.pending = again;
    Ok(())
}

/// A new inline panel of the height the command list needs; ratatui can't resize one. When the
/// panel shrinks at the bottom of the screen, what is above it moves down with it, so the freed
/// rows end up at the top of the screen and not between the messages and the panel.
fn resize(term: &mut Term, app: &mut App) -> io::Result<()> {
    if app.more && app.rustc.is_none() {
        app.rustc = Some(rustc_version());
    }
    let old = term.get_frame().area();
    let screen = term.size()?.height;
    let new = app.height();
    term.clear()?;
    if new < old.height && old.y + old.height >= screen {
        let top = screen.saturating_sub(new);
        let mut out = stdout();
        write!(out, "\x1b[1;{top}r\x1b[{}T\x1b[r", old.height - new)?;
        out.execute(MoveTo(0, top))?;
    }
    *term = open(new)?;
    Ok(())
}

fn rustc_version() -> String {
    Command::new("rustc")
        .arg("--version")
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().trim_start_matches("rustc ").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

fn open_url(url: &str) -> io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        Command::new("open")
    } else if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.args(["/C", "start", ""]);
        c
    } else {
        Command::new("xdg-open")
    };
    cmd.arg(url).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()?;
    Ok(())
}

/// Reads terminal events on its own thread. It polls instead of blocking in `read`, so the
/// terminal can ask where the cursor is (when the panel changes height) without waiting for a key.
fn spawn_input(pipe: &Pipeline) {
    let tx = pipe.tx.clone();
    std::thread::Builder::new()
        .name("yawn-input".into())
        .spawn(move || {
            loop {
                match event::poll(Duration::from_millis(50)) {
                    Ok(true) => match event::read() {
                        Ok(ev) => {
                            if tx.send(Msg::Input(ev)).is_err() {
                                break;
                            }
                        }
                        Err(_) => break,
                    },
                    Ok(false) => {}
                    Err(_) => break,
                }
            }
        })
        .expect("spawn input thread");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{Event, LogEvent};
    use app::tests::{app, key};
    use ratatui::crossterm::event::KeyCode;

    #[test]
    fn events_redraw_resizes_reflow_but_focus_does_not() {
        let mut a = app();
        let ev = Event::Log(LogEvent::new(Duration::ZERO, Level::Info, "s", "x"));
        assert_eq!(handle(&mut a, Msg::Event(ev)), Act::Redraw);
        assert_eq!(handle(&mut a, Msg::Input(TermEvent::Resize(80, 24))), Act::Reflow);
        assert_eq!(handle(&mut a, Msg::Input(TermEvent::FocusGained)), Act::Nothing);
        assert_eq!(handle(&mut a, Msg::Input(TermEvent::Key(key(KeyCode::F(9))))), Act::Nothing);
        assert_eq!(a.pending.len(), 1);
    }

    #[test]
    fn signal_quits_and_source_end_is_recorded() {
        let mut a = app();
        handle(&mut a, Msg::SourceEnded);
        assert!(a.state.ended() && !a.quit);
        handle(&mut a, Msg::Signal);
        assert!(a.quit);
    }
}
