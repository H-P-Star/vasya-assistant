//! Developer probe for PCM 16 kHz mono WAV; never executes recognized commands.
use anyhow::{Context, Result};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.len() != 6 {
        anyhow::bail!(
            "Usage: check_recognition <libvosk> <vosk model> <16k wav> <whisper-cli> <ggml model>"
        );
    }
    let bytes = std::fs::read(&args[3])?;
    let mut at = 12;
    let mut samples = None;
    while at + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into()?) as usize;
        let start = at + 8;
        let end = (start + size).min(bytes.len());
        if &bytes[at..at + 4] == b"data" {
            samples = Some(
                bytes[start..end]
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect::<Vec<_>>(),
            );
            break;
        }
        at = start + size + size % 2;
    }
    let samples = samples.context("No WAV data chunk")?;
    let mut vosk = vasya::speech::Vosk::load(
        std::path::Path::new(&args[1]),
        std::path::Path::new(&args[2]),
        16000.0,
    )?;
    let words = vec!["вась".to_string(), "вася".to_string()];
    vosk.configure_wake(&words, std::path::Path::new(&args[2]))?;
    let mut transcript = Vec::new();
    let mut detected = false;
    for chunk in samples.chunks(1600) {
        let wake = vosk.wake(chunk)?;
        detected |= vasya::activation::strip_wake(&wake, &words).is_some();
        if let Some(text) = vosk.accept(chunk)? {
            transcript.push(text);
        }
    }
    transcript.push(vosk.finish()?);
    let settings = vasya::config::Settings {
        whisper_executable: args[4].clone(),
        whisper_model: args[5].clone(),
        speech_language: "auto".into(),
        speech_hint: "Вася, Вась. Linkin Park, Numb, Кино, Перемен.".into(),
        ..Default::default()
    };
    println!(
        "{}",
        serde_json::json!({"wake_detected":detected,"vosk":transcript.join(" ").trim()})
    );
    let started = std::time::Instant::now();
    let refined = vasya::speech::whisper_configured(&settings, &samples, || false)?;
    println!(
        "{}",
        serde_json::json!({"wake_detected":detected,"vosk":transcript.join(" ").trim(),"whisper":refined,"whisper_seconds":started.elapsed().as_secs_f32()})
    );
    Ok(())
}
