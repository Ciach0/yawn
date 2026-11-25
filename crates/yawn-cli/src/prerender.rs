//! `yawn build --static`: the built server renders every page once, and the HTML (with the data the
//! pages boot with) is written next to the hashed files, ready for a static host.

use crate::error::CliError;
use std::io::{Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// What was written
pub struct Written {
    pub pages: usize,
    pub data: usize,
}

/// Checks a page from `[build] prerender`: a path of its own, nothing that escapes the output
pub fn check_page(path: &str) -> Result<(), String> {
    if !path.starts_with('/') || path.starts_with("//") || path.contains(['?', '#']) {
        return Err(format!("[build] prerender: `{path}` must be a path like `/users/1`"));
    }
    if path.contains('\\') || path.split('/').any(|s| s == "." || s == "..") {
        return Err(format!("[build] prerender: `{path}` cannot contain `.`, `..` or `\\`"));
    }
    Ok(())
}

/// Where the HTML of `page` goes: `/` is `index.html`, `/about` is `about/index.html`, which every
/// static host serves at `/about`
pub fn html_file(public: &Path, page: &str) -> PathBuf {
    page.split('/')
        .filter(|s| !s.is_empty())
        .fold(public.to_path_buf(), |p, s| p.join(s))
        .join("index.html")
}

/// `href`s of the `<link data-yawn-boot>` tags in a page: the data the client fetches as it starts
pub fn boot_urls(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in html.split("<link ").skip(1) {
        let tag = tag.split('>').next().unwrap_or("");
        if !tag.contains(" data-yawn-boot=") {
            continue;
        }
        let Some(href) = tag.split(" href=\"").nth(1).and_then(|h| h.split('"').next()) else {
            continue;
        };
        let href = href.replace("&quot;", "\"").replace("&lt;", "<").replace("&amp;", "&");
        if !out.contains(&href) {
            out.push(href);
        }
    }
    out
}

/// Status and body of an HTTP/1.0 response (1.0, so the body is never chunked)
pub fn parse_response(raw: &[u8]) -> Option<(u16, &[u8])> {
    let end = raw.windows(4).position(|w| w == b"\r\n\r\n")?;
    let head = std::str::from_utf8(&raw[..end]).ok()?;
    let status = head.split_whitespace().nth(1)?.parse().ok()?;
    Some((status, &raw[end + 4..]))
}

fn get(addr: SocketAddr, path: &str) -> std::io::Result<(u16, Vec<u8>)> {
    let mut s = TcpStream::connect(addr)?;
    s.set_read_timeout(Some(Duration::from_secs(60)))?;
    write!(s, "GET {path} HTTP/1.0\r\nHost: {addr}\r\n\r\n")?;
    let mut raw = Vec::new();
    s.read_to_end(&mut raw)?;
    parse_response(&raw)
        .map(|(status, body)| (status, body.to_vec()))
        .ok_or_else(|| std::io::Error::other("not an HTTP response"))
}

struct Server(Child);

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn start(
    exe: &Path,
    root: &Path,
    public: &Path,
    host: Option<IpAddr>,
) -> Result<(Server, SocketAddr), CliError> {
    let ip = host.filter(|h| !h.is_unspecified()).unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
    // shortcut: the port could be taken between this probe and the server's bind; rare on loopback
    let addr = TcpListener::bind((ip, 0)).and_then(|l| l.local_addr())?;
    let child = Command::new(exe)
        .current_dir(root)
        .env("YAWN_PUBLIC", public)
        .env("YAWN_PORT", addr.port().to_string())
        .env_remove("YAWN_DEV")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| CliError::new(format!("cannot start {}: {e}", exe.display())))?;
    let mut server = Server(child);
    let started = Instant::now();
    while TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_err() {
        if let Some(status) = server.0.try_wait()? {
            return Err(CliError::new(format!("the server exited before it served ({status})")));
        }
        if started.elapsed() > Duration::from_secs(30) {
            return Err(CliError::new(format!("the server did not start listening on {addr}")));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok((server, addr))
}

fn write(file: &Path, body: &[u8]) -> Result<(), CliError> {
    std::fs::create_dir_all(file.parent().expect("below public"))
        .and_then(|()| std::fs::write(file, body))
        .map_err(|e| CliError::new(format!("cannot write {}: {e}", file.display())))
}

/// Renders `pages` with the server `exe` and writes them, and the data they boot with, into `public`
pub fn run(
    exe: &Path,
    root: &Path,
    public: &Path,
    host: Option<IpAddr>,
    pages: &[String],
) -> Result<Written, CliError> {
    let (_server, addr) = start(exe, root, public, host)?;
    let mut data = Vec::new();
    for page in pages {
        let (status, body) =
            get(addr, page).map_err(|e| CliError::new(format!("cannot render {page}: {e}")))?;
        if status != 200 {
            return Err(CliError::new(format!("{page} answered {status}")));
        }
        for url in boot_urls(&String::from_utf8_lossy(&body)) {
            if !data.contains(&url) {
                data.push(url);
            }
        }
        write(&html_file(public, page), &body)?;
    }
    let mut saved = 0;
    for url in &data {
        if !url.starts_with('/') || url.starts_with("//") {
            continue; // another site: the browser fetches it from there
        }
        if url.contains(['?', '#', '\\']) || url.ends_with('/') || url.split('/').any(|s| s == "..")
        {
            eprintln!(
                "      warning: {url} can't be a file on a static host; pages that read it get no data"
            );
            continue;
        }
        let file = public.join(url.trim_start_matches('/'));
        if file.is_file() {
            continue; // an asset, already there
        }
        if file.is_dir() {
            eprintln!("      warning: {url} is also the folder of another data URL; not saved");
            continue;
        }
        match get(addr, url) {
            Ok((200, body)) => match write(&file, &body) {
                Ok(()) => saved += 1,
                Err(_) => eprintln!(
                    "      warning: {url} clashes with another data URL (a file and a folder with one name); not saved"
                ),
            },
            Ok((status, _)) => eprintln!("      warning: {url} answered {status}; not saved"),
            Err(e) => eprintln!("      warning: cannot fetch {url}: {e}"),
        }
    }
    Ok(Written { pages: pages.len(), data: saved })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pages_are_checked() {
        assert!(check_page("/users/1").is_ok());
        assert!(check_page("/").is_ok());
        for bad in ["users", "//evil.com", "/a?b=1", "/a#x", "/../etc", "/a/./b", "/a\\..\\b"] {
            assert!(check_page(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn html_files() {
        let p = Path::new("/out");
        assert_eq!(html_file(p, "/"), Path::new("/out/index.html"));
        assert_eq!(html_file(p, "/about"), Path::new("/out/about/index.html"));
        assert_eq!(html_file(p, "/users/1/"), Path::new("/out/users/1/index.html"));
    }

    #[test]
    fn boot_links_are_found() {
        let html = "<link rel=\"stylesheet\" href=\"/a.css\">\n\
            <link rel=\"preload\" as=\"fetch\" crossorigin=\"\" href=\"/api/user?id=1&amp;x=2\" data-yawn-boot=\"user\">\n\
            <link rel=\"preload\" as=\"fetch\" crossorigin=\"\" href=\"/api/list\" data-yawn-boot=\"list\">\n\
            <link rel=\"preload\" as=\"fetch\" crossorigin=\"\" href=\"/api/list\" data-yawn-boot=\"again\">\n";
        assert_eq!(boot_urls(html), ["/api/user?id=1&x=2", "/api/list"]);
    }

    #[test]
    fn responses_are_parsed() {
        let raw = b"HTTP/1.0 200 OK\r\ncontent-type: text/html\r\n\r\n<p>hi</p>";
        assert_eq!(parse_response(raw), Some((200, &b"<p>hi</p>"[..])));
        assert_eq!(parse_response(b"HTTP/1.0 404 Not Found\r\n\r\n"), Some((404, &b""[..])));
        assert_eq!(parse_response(b"garbage"), None);
    }
}
