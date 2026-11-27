fn main() {
    println!("cargo:rerun-if-changed=capabilities/main.json");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "main_bootstrap",
            "list_messages",
            "open_message",
            "set_reader_layout",
            "update_main_preferences",
            "refresh_mail",
        ]),
    ))
    .expect("failed to run tauri build");
}
