// Server functions next to a component that calls one.
use std::fmt::Write as _;

/// Adds on the server
server fn add(a: i32, mut b: i32) -> i32 {
    b += a;
    b
}

pub server fn ping() {}

server fn describe(words: Vec<String>, sep: char) -> Result<String, String> {
    if words.is_empty() {
        return Err("no words".into());
    }
    let mut out = String::new();
    for w in &words {
        let _ = write!(out, "{w}{sep}");
    }
    Ok(out)
}

component Adder() {
    let total = signal(0);

    view {
        <button on:click={move || yawn::task::spawn(add(total.get(), 1), move |r| total.set(r.unwrap_or(-1)))}>"{total}"</button>
    }
}
