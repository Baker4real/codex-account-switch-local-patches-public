#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("codex_switch supports Windows only");

fn main() {
    let args = std::env::args().collect::<Vec<_>>();
    if args.len() > 1 {
        std::process::exit(codex_switch_lib::run_cli(&args[1..]));
    }

    codex_switch_lib::run();
}
