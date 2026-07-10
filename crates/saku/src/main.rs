//! Saku binary entrypoint.
//!
//! `saku` runs the Discord bot; `saku login` delegates to `saku-cli`.

use std::env;
use std::process::ExitCode;

#[tokio::main]
async fn main() -> ExitCode {
    let mut args = env::args().skip(1).collect::<Vec<_>>();
    if args.first().map(String::as_str) == Some("login") {
        args.remove(0);
        return match args.as_slice() {
            [cmd] if cmd == "codex" => match saku_cli::login_codex(None).await {
                Ok(()) => ExitCode::SUCCESS,
                Err(err) => {
                    eprintln!("saku login codex: {err}");
                    ExitCode::FAILURE
                }
            },
            _ => {
                eprintln!("usage: saku login codex");
                ExitCode::FAILURE
            }
        };
    }

    // Discord bot wiring lands in ticket #10.
    eprintln!("saku: Discord bot not yet started — use `saku login codex` or see issue #10");
    ExitCode::FAILURE
}
