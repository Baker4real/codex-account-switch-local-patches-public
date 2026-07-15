//! Dedicated local proxy used only by the Codex Switch process tree.

use std::process::Command;

pub const CODEX_SWITCH_PROXY_URL: &str = "http://127.0.0.1:10809";

pub fn configure_child_process(command: &mut Command) {
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(name, CODEX_SWITCH_PROXY_URL);
    }
}
