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
            "builtin_commands",
            "save_api_key",
            "list_voices",
            "calibrate_speech",
            "dismiss_island",
            "expand_island",
            "show_last_result",
            "show_main",
            "copy_result",
            "test_voice",
            "restart_music_app",
            "cancel_current",
            "clear_conversation",
            "open_answer_link",
            "open_resource",
        ]),
    ))
    .expect("Не удалось подготовить ресурсы Tauri");
}
