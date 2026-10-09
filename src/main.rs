// Entry point: route a command line to the session commands, the server, the integrations, or the API CLI (cli/).
// Everything runs on one thread: a current-thread tokio runtime and a LocalSet.
mod cli;
mod client;
mod config;
mod core;
mod integrations;
mod platform;
mod protocol;
mod server;
mod skills;
mod vt;

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    if argv.first().map(String::as_str) == Some("__pty-exec") {
        pty_exec(&argv[1..]);
    }
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("tokio runtime");
    let code = tokio::task::LocalSet::new().block_on(&rt, cli::run(argv));
    std::process::exit(code);
}

// `modisa __pty-exec <cmd…>`: claim the PTY on stdin as the controlling terminal, then become <cmd>. Servers of the
// TypeScript build (up to 0.2.1) start every pane through this; this build's own panes don't need it.
// ponytail: drop once no TypeScript-build server can still be running
fn pty_exec(argv: &[String]) -> ! {
    use std::os::unix::process::CommandExt;
    unsafe { libc::ioctl(0, libc::TIOCSCTTY as _, 0) };
    let Some((cmd, args)) = argv.split_first() else { std::process::exit(2) };
    let e = std::process::Command::new(cmd).args(args).exec();
    eprintln!("modisa: could not start {cmd}: {e}");
    std::process::exit(127);
}
