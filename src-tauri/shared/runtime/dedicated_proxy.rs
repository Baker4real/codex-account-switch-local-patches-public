//! Dedicated local proxy used only by the Codex Switch process tree.

use std::{env, process::Command};

pub const DEFAULT_CODEX_SWITCH_PROXY_URL: &str = "http://127.0.0.1:10808";

const PROXY_ENV_NAMES: [&str; 7] = [
    "CODEX_SWITCH_PROXY_URL",
    "HTTPS_PROXY",
    "https_proxy",
    "ALL_PROXY",
    "all_proxy",
    "HTTP_PROXY",
    "http_proxy",
];

fn resolve_proxy_url_from(mut read: impl FnMut(&str) -> Option<String>) -> String {
    PROXY_ENV_NAMES
        .iter()
        .find_map(|name| {
            let value = read(name)?;
            let value = value.trim();
            (!value.is_empty()).then(|| value.to_string())
        })
        .unwrap_or_else(|| DEFAULT_CODEX_SWITCH_PROXY_URL.to_string())
}

pub fn proxy_url() -> String {
    resolve_proxy_url_from(|name| env::var(name).ok())
}

pub fn configure_child_process(command: &mut Command) {
    let proxy_url = proxy_url();
    for name in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        command.env(name, &proxy_url);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn resolve_with(values: &[(&str, &str)]) -> String {
        let values = values
            .iter()
            .map(|(name, value)| ((*name).to_string(), (*value).to_string()))
            .collect::<HashMap<_, _>>();
        resolve_proxy_url_from(|name| values.get(name).cloned())
    }

    #[test]
    fn explicit_switcher_proxy_override_wins() {
        assert_eq!(
            resolve_with(&[
                ("CODEX_SWITCH_PROXY_URL", "http://127.0.0.1:19090"),
                ("HTTPS_PROXY", "http://127.0.0.1:10808"),
            ]),
            "http://127.0.0.1:19090"
        );
    }

    #[test]
    fn existing_https_proxy_beats_the_compiled_fallback() {
        assert_eq!(
            resolve_with(&[("HTTPS_PROXY", "http://127.0.0.1:10808")]),
            "http://127.0.0.1:10808"
        );
    }

    #[test]
    fn missing_or_blank_environment_uses_the_local_default() {
        assert_eq!(
            resolve_with(&[("HTTPS_PROXY", "   ")]),
            DEFAULT_CODEX_SWITCH_PROXY_URL
        );
    }
}
