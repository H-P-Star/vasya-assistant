fn main() {
    #[cfg(all(windows, feature = "desktop"))]
    tauri_build::try_build(tauri_build::Attributes::new().app_manifest(
        tauri_build::AppManifest::new().commands(&[
            "get_state",
            "list_microphones",
            "test_microphone",
            "finish_setup",
            "check_llm",
            "set_muted",
            "activate",
            "run_text",
            "open_music_login",
            "insert_result",
            "save_settings",
        ]),
    ))
    .expect("Не удалось подготовить ресурсы Tauri");
}
