#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    #[cfg(all(windows, feature = "desktop"))]
    if std::env::args().nth(1).as_deref() == Some("--self-test-output") {
        let Some(output) = std::env::args_os().nth(2) else {
            std::process::exit(2);
        };
        let result = (|| -> anyhow::Result<()> {
            let exe = std::env::current_exe()?;
            let settings = vasya::config::resolve_speech(
                &vasya::config::Settings::default(),
                exe.parent().unwrap(),
            );
            let mut vosk = vasya::speech::Vosk::load(
                std::path::Path::new(&settings.vosk_library),
                std::path::Path::new(&settings.vosk_model),
                16000.0,
            )?;
            vosk.accept(&vec![0i16; 16000])?;
            Ok(())
        })();
        let report = serde_json::json!({"ok":result.is_ok(), "error":result.as_ref().err().map(|e| format!("{e:#}"))});
        let _ = std::fs::write(output, report.to_string());
        std::process::exit(if result.is_ok() { 0 } else { 1 });
    }
    #[cfg(all(windows, feature = "desktop"))]
    vasya::desktop::run();
    #[cfg(not(windows))]
    eprintln!("Vasya desktop requires Windows 10/11. Core tests can run on this platform.");
}
