use crate::config::Settings;
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    fs::{self, File},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[derive(Serialize)]
pub struct Voice {
    pub id: String,
    pub name: String,
}

fn model_argument(model: &Path, config: &Path, work_dir: &Path) -> Result<PathBuf> {
    if model.as_os_str().as_encoded_bytes().is_ascii() {
        return Ok(model.to_path_buf());
    }

    // Legacy Piper widens the bytes of the ONNX path one by one on Windows.
    // Relative ASCII names also work when the temporary directory contains Cyrillic.
    for (source, name) in [(model, "voice.onnx"), (config, "voice.onnx.json")] {
        let target = work_dir.join(name);
        fs::hard_link(source, &target)
            .or_else(|_| fs::copy(source, &target).map(|_| ()))
            .with_context(|| {
                format!(
                    "Не удалось подготовить файл голоса «{}» для Piper",
                    source.display()
                )
            })?;
    }
    Ok(PathBuf::from("voice.onnx"))
}

fn diagnostic(log: &mut File) -> Option<String> {
    let offset = log.metadata().ok()?.len().saturating_sub(4096);
    log.seek(SeekFrom::Start(offset)).ok()?;
    let mut bytes = Vec::new();
    log.take(4096).read_to_end(&mut bytes).ok()?;
    let detail: String = String::from_utf8_lossy(&bytes)
        .lines()
        .rev()
        .find(|line| !line.trim().is_empty())?
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(500)
        .collect();
    (!detail.is_empty()).then_some(detail)
}

pub fn synthesize(
    settings: &Settings,
    text: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let exe = Path::new(&settings.piper_executable);
    let model = Path::new(&settings.piper_model);
    let config = model.with_extension("onnx.json");
    if !exe.is_file() {
        bail!("Не найден piper.exe по пути «{}»", exe.display());
    }
    if !model.is_file() {
        bail!("Не найден файл модели Piper «{}»", model.display());
    }
    if !config.is_file() {
        bail!("Не найден файл настроек голоса «{}». Скачайте .onnx.json той же модели и положите рядом с .onnx", config.display());
    }
    if cancelled() {
        bail!("Озвучивание отменено");
    }

    let exe = dunce::canonicalize(exe)?;
    let model = std::path::absolute(model)?;
    let config = std::path::absolute(config)?;
    let dir = tempfile::tempdir()?;
    let model = model_argument(&model, &config, dir.path())?;
    let mut log = tempfile::tempfile()?;
    let mut command = Command::new(exe);
    command
        .current_dir(dir.path())
        .arg("--model")
        .arg(model)
        .arg("--output_file")
        .arg("voice.wav")
        .arg("--speaker")
        .arg(settings.piper_speaker.to_string())
        .arg("--length_scale")
        .arg(2f32.powf(-(settings.tts_rate as f32) / 10.0).to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(log.try_clone()?);
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
                let detail = diagnostic(&mut log);
                if let Some(detail) = detail {
                    bail!("Piper завершился с ошибкой ({status}). {detail}");
                }
                if status.code().is_some_and(|code| code as u32 == 0xC0000135) {
                    bail!("Не найдена DLL для Piper. Распакуйте весь архив Piper рядом с piper.exe. Если DLL уже на месте, проверьте Microsoft Visual C++ Redistributable x64");
                }
                bail!("Piper завершился с ошибкой ({status}). Проверьте комплект DLL и совместимость .onnx и .onnx.json с выбранной версией Piper");
            }
            break;
        }
        std::thread::sleep(Duration::from_millis(40));
    }
    writer
        .join()
        .map_err(|_| anyhow::anyhow!("Не удалось передать текст в Piper"))??;
    let mut bytes = fs::read(dir.path().join("voice.wav")).context(
        "Piper завершил работу без звукового файла. Проверьте версию piper.exe и модель голоса",
    )?;
    scale_wav(&mut bytes, settings.tts_volume)?;
    Ok(bytes)
}

const FISH_TTS_URL: &str = "https://api.fish.audio/v1/tts";
const FISH_SAMPLE_RATE: u32 = 24_000;
const MAX_FISH_AUDIO_BYTES: usize = 16 * 1024 * 1024;

fn fish_reference(value: &str, model: &str) -> Result<String> {
    if !["s2.1-pro-free", "s2.1-pro", "s2-pro"].contains(&model) {
        bail!("Выберите модель синтеза Fish Audio");
    }
    let value = value.trim();
    let id = if let Ok(url) = url::Url::parse(value) {
        if url.scheme() != "https"
            || !matches!(url.host_str(), Some("fish.audio" | "www.fish.audio"))
        {
            bail!("Укажите ссылку на голос на сайте fish.audio или его ID");
        }
        let parts: Vec<_> = url.path().trim_matches('/').split('/').collect();
        match parts.as_slice() {
            ["m", id] | ["app", "m", id] => (*id).to_owned(),
            _ => bail!("Скопируйте ссылку на конкретный голос Fish Audio или его ID"),
        }
    } else {
        value.to_owned()
    };
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
    {
        bail!("Укажите ID голоса Fish Audio или ссылку на его страницу");
    }
    Ok(id)
}

pub(crate) fn validate_fish_voice(voice: &str, model: &str) -> Result<()> {
    fish_reference(voice, model).map(drop)
}

pub fn synthesize_fish(
    settings: &Settings,
    text: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let key = crate::credentials::load("fish_audio")?
        .filter(|key| !key.is_empty())
        .context("Сохраните API-ключ Fish Audio в разделе «Голос Васи»")?;
    request_fish(FISH_TTS_URL, &key, settings, text, cancelled)
}

fn request_fish(
    endpoint: &str,
    key: &str,
    settings: &Settings,
    text: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<u8>> {
    let id = fish_reference(&settings.fish_voice, &settings.fish_model)?;
    let body = serde_json::json!({
        "text": text,
        "reference_id": id,
        "format": "pcm",
        "sample_rate": FISH_SAMPLE_RATE,
        "prosody": { "speed": 2f32.powf(settings.tts_rate as f32 / 10.0) },
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let mut wav = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .build()?;
        let mut request = Box::pin(async {
            let mut response = client
                .post(endpoint)
                .bearer_auth(key)
                .header("model", settings.fish_model.as_str())
                .json(&body)
                .send()
                .await
                .context("Не удалось связаться с Fish Audio")?;
            match response.status().as_u16() {
                200 => {}
                401 => bail!("Fish Audio отклонил API-ключ. Проверьте ключ в настройках"),
                402 => bail!("Fish Audio требует оплату для этого запроса. Проверьте API-баланс или выберите бесплатную модель"),
                403 | 404 => bail!("Fish Audio не предоставил доступ к голосу. Проверьте его ID и доступность для вашего ключа"),
                429 => bail!("Достигнут лимит запросов Fish Audio. Повторите позже"),
                status => bail!("Fish Audio вернул HTTP {status}. Проверьте голос и модель синтеза"),
            }
            let mut wav = vec![0; 44];
            while let Some(chunk) = response.chunk().await.context("Загрузка звука Fish Audio прервана")? {
                if wav.len() + chunk.len() > 44 + MAX_FISH_AUDIO_BYTES {
                    bail!("Звуковой ответ Fish Audio слишком большой");
                }
                wav.extend_from_slice(&chunk);
            }
            let size = wav.len() - 44;
            if size == 0 || !size.is_multiple_of(2) {
                bail!("Fish Audio вернул пустой или неполный PCM");
            }
            wav[..44].copy_from_slice(&crate::speech::pcm_wav_header(size as u32, FISH_SAMPLE_RATE));
            Ok(wav)
        });
        loop {
            if cancelled() {
                bail!("Озвучивание отменено");
            }
            if let Ok(result) = tokio::time::timeout(Duration::from_millis(50), &mut request).await {
                return result;
            }
        }
    })?;
    scale_wav(&mut wav, settings.tts_volume)?;
    Ok(wav)
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
    fn fish_audio_sends_the_selected_voice_and_wraps_pcm_for_playback() {
        use std::{
            io::{BufRead, BufReader},
            net::TcpListener,
        };

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint = format!("http://{}/v1/tts", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut socket, _) = listener.accept().unwrap();
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut reader = BufReader::new(&mut socket);
            let mut headers = Vec::new();
            loop {
                let mut line = String::new();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                headers.push(line.trim().to_ascii_lowercase());
            }
            assert_eq!(headers[0], "post /v1/tts http/1.1");
            assert!(headers
                .iter()
                .any(|h| h == "authorization: bearer test-key"));
            assert!(headers.iter().any(|h| h == "model: s2.1-pro-free"));
            let size = headers
                .iter()
                .find_map(|h| h.strip_prefix("content-length: "))
                .unwrap();
            let mut body = vec![0; size.parse().unwrap()];
            reader.read_exact(&mut body).unwrap();
            let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
            assert_eq!(body["reference_id"], "3e18ecf10d624b01a936dcc3308e8dfa");
            assert_eq!(body["text"], "Привет!");
            assert_eq!(body["format"], "pcm");
            assert_eq!(body["sample_rate"], FISH_SAMPLE_RATE);
            assert_eq!(body["prosody"]["speed"], 2.0);
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\n")
                .unwrap();
            socket.write_all(&16000i16.to_le_bytes()).unwrap();
            socket.write_all(&(-16000i16).to_le_bytes()).unwrap();
        });
        let settings = Settings {
            fish_voice: "https://fish.audio/m/3e18ecf10d624b01a936dcc3308e8dfa/?ref=example".into(),
            tts_rate: 10,
            tts_volume: 50,
            ..Settings::default()
        };
        let wav = request_fish(&endpoint, "test-key", &settings, "Привет!", &|| false).unwrap();
        assert_eq!(
            wav[..44],
            crate::speech::pcm_wav_header(4, FISH_SAMPLE_RATE)
        );
        assert_eq!(&wav[44..46], &8000i16.to_le_bytes());
        assert_eq!(&wav[46..48], &(-8000i16).to_le_bytes());
        server.join().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn piper_passes_ascii_arguments_for_a_cyrillic_voice_and_reports_stderr() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("piper");
        let voices = dir.path().join("дмитрий_какой-то");
        fs::create_dir(&voices).unwrap();
        let model = voices.join("ru_RU-dmitri-medium.onnx");
        fs::write(&model, b"model").unwrap();
        fs::write(model.with_extension("onnx.json"), b"{}").unwrap();
        fs::write(
            &exe,
            b"#!/bin/sh\nread -r text\n[ \"$2\" = voice.onnx ] && [ \"$4\" = voice.wav ] && [ -f voice.onnx ] && [ -f voice.onnx.json ] || exit 2\nprintf 'voice config is incompatible\\n' >&2\nexit 1\n",
        )
        .unwrap();
        fs::set_permissions(&exe, fs::Permissions::from_mode(0o700)).unwrap();
        let settings = Settings {
            piper_executable: exe.to_string_lossy().into(),
            piper_model: model.to_string_lossy().into(),
            ..Settings::default()
        };
        let error = synthesize(&settings, "Привет", &|| false).unwrap_err();
        assert!(error.to_string().contains("voice config is incompatible"));
        assert!(!voices.join("voice.wav").exists());
    }

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
