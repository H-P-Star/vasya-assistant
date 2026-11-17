use crate::config::Settings;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    io::Write,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

pub fn synthesize(
    settings: &Settings,
    text: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let exe = std::path::Path::new(&settings.piper_executable);
    let model = std::path::Path::new(&settings.piper_model);
    if !exe.is_file() || !model.is_file() || !model.with_extension("onnx.json").is_file() {
        bail!("Для Piper укажите piper.exe, файл .onnx и расположите рядом его .onnx.json");
    }
    let dir = tempfile::tempdir()?;
    let wav = dir.path().join("voice.wav");
    let mut command = Command::new(exe);
    command
        .arg("--model")
        .arg(model)
        .arg("--output_file")
        .arg(&wav)
        .arg("--speaker")
        .arg(settings.piper_speaker.to_string())
        .arg("--length_scale")
        .arg(2f32.powf(-(settings.tts_rate as f32) / 10.0).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .context("Не удалось запустить Piper. Проверьте .exe и DLL из его архива")?;
    let mut input = child.stdin.take().context("Piper не открыл ввод текста")?;
    let text = text.replace(['\r', '\n'], " ");
    // A blocked stdin pipe must not prevent cancellation of a broken voice engine.
    let writer = std::thread::spawn(move || writeln!(input, "{text}"));
    let started = Instant::now();
    loop {
        if cancelled() || started.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            let _ = writer.join();
            bail!("Озвучивание отменено или превышено время синтеза");
        }
        let status = match child.try_wait() {
            Ok(status) => status,
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = writer.join();
                return Err(error.into());
            }
        };
        if let Some(status) = status {
            if !status.success() {
                let _ = writer.join();
                bail!("Piper завершился с ошибкой. Проверьте номер диктора и совместимость: для piper.exe 2023 нужны модель и конфигурация из каталога голосов v1.0.0");
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("Не удалось передать текст в Piper"))??;
    let mut bytes = std::fs::read(wav)?;
    scale_wav(&mut bytes, settings.tts_volume)?;
    Ok(bytes)
}

/// Scale only the generated PCM, keeping Windows and music volume unchanged.
pub fn scale_wav(bytes: &mut [u8], volume: u16) -> Result<Duration> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        bail!("Голос вернул некорректный WAV");
    }
    let mut offset = 12;
    let mut rate = 0;
    let mut pcm16 = false;
    let mut data = None;
    while offset + 8 <= bytes.len() {
        let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into()?) as usize;
        let start = offset + 8;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .context("Повреждённый WAV голоса")?;
        if &bytes[offset..offset + 4] == b"fmt " && size >= 16 {
            pcm16 = u16::from_le_bytes(bytes[start..start + 2].try_into()?) == 1
                && u16::from_le_bytes(bytes[start + 14..start + 16].try_into()?) == 16;
            rate = u32::from_le_bytes(bytes[start + 8..start + 12].try_into()?);
        }
        if &bytes[offset..offset + 4] == b"data" {
            data = Some(start..end);
        }
        offset = end + size % 2;
    }
    let data = data.context("В WAV нет звуковых данных")?;
    if !pcm16 || rate == 0 || data.len() % 2 != 0 {
        bail!("Голос должен возвращать WAV PCM 16 bit");
    }
    let duration = Duration::from_secs_f64(data.len() as f64 / rate as f64);
    for sample in bytes[data].as_chunks_mut::<2>().0 {
        let value = i16::from_le_bytes([sample[0], sample[1]]) as i32;
        sample.copy_from_slice(&((value * volume.min(100) as i32 / 100) as i16).to_le_bytes());
    }
    Ok(duration)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn voice_volume_changes_only_pcm_and_rejects_truncated_chunks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("voice.wav");
        crate::speech::write_wav(&path, &[16000, -16000]).unwrap();
        let mut bytes = std::fs::read(path).unwrap();
        let header = bytes[..44].to_vec();
        let duration = scale_wav(&mut bytes, 25).unwrap();
        assert_eq!(&bytes[..44], header);
        assert_eq!(i16::from_le_bytes(bytes[44..46].try_into().unwrap()), 4000);
        assert_eq!(i16::from_le_bytes(bytes[46..48].try_into().unwrap()), -4000);
        assert!(duration.as_secs_f32() > 0.0);
        assert!(scale_wav(&mut bytes[..47], 100).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn cancelling_a_voice_engine_also_unblocks_its_full_stdin_pipe() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("piper");
        let model = dir.path().join("voice.onnx");
        std::fs::write(&exe, b"#!/bin/sh\nexec /bin/sleep 30\n").unwrap();
        std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o700)).unwrap();
        std::fs::write(&model, b"test").unwrap();
        std::fs::write(model.with_extension("onnx.json"), b"{}").unwrap();
        let settings = Settings {
            piper_executable: exe.to_string_lossy().into(),
            piper_model: model.to_string_lossy().into(),
            ..Default::default()
        };
        let start = Instant::now();
        let result = synthesize(&settings, &"x".repeat(100000), &|| {
            start.elapsed() > Duration::from_millis(100)
        });
        assert!(result.unwrap_err().to_string().contains("отменено"));
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
