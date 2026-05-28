//! Verbreel — CLI binary entrypoint.
//!
//! All routing lives in `verbreel_cli::dispatch`. This binary is a thin
//! wrapper so integration tests can call the library function directly
//! without spawning subprocesses.

use std::io::{self, Write as _};

use verbreel_cli::dispatch;

fn main() {
    tracing_subscriber::fmt::init();

    let mut stdout = io::stdout().lock();
    let mut stderr = io::stderr().lock();
    let code = dispatch(std::env::args_os(), &mut stdout, &mut stderr);
    let _ = stdout.flush();
    let _ = stderr.flush();
    std::process::exit(code);
}
