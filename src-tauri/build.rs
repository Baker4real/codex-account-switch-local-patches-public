use std::env;
use std::path::PathBuf;

fn main() {
    if env::var_os("CARGO_CFG_WINDOWS").is_some()
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu")
    {
        println!("cargo:rustc-cdylib-link-arg=-Wl,--exclude-all-symbols");
    }

    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source_cli_path = manifest_dir
        .join("target")
        .join("release")
        .join("codex_switch.exe");

    if source_cli_path.exists() {
        println!(
            "cargo:rustc-env=CODEX_SWITCH_RELEASE_EXE={}",
            source_cli_path.display()
        );
    }

    tauri_build::build()
}
