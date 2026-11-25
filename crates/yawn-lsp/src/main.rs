mod complete;
mod context;
mod format;
mod index;
mod mask;
mod ontype;
mod position;
pub mod server;
mod symbols;
mod tailwind;
mod trace;
mod web;

fn main() {
    if let Err(e) = server::run() {
        eprintln!("yawn-lsp: {e}");
        std::process::exit(1);
    }
}
