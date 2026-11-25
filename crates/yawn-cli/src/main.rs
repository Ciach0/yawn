fn main() -> std::process::ExitCode {
    yawn_cli::cli::run(std::env::args_os())
}
