use crate::{
    activation::{confirm_restricted_wake, strip_wake, Accepted, Gate},
    config::{Activation, Settings, SpeechMode},
    runtime::{Shared, Task},
    speech::Vosk,
};
use anyhow::{bail, Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::{
    path::Path,
    sync::{
        atomic::Ordering::SeqCst,
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    time::{Duration, Instant},
};

pub fn devices() -> Result<Vec<String>> {
    Ok(cpal::default_host()
        .input_devices()?
        .filter_map(|d| d.name().ok())
        .collect())
}

fn capture(name: &str, seconds: u64, cancelled: impl Fn() -> bool) -> Result<Vec<i16>> {
    let host = cpal::default_host();
    let device = if name.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|d| d.name().ok().as_deref() == Some(name))
    }
    .context("Микрофон калибровки не найден")?;
    let supported = device.default_input_config()?;
    let config: cpal::StreamConfig = supported.clone().into();
    let (tx, rx) = mpsc::sync_channel(96);
    let failed = Arc::new(std::sync::Mutex::new(None));
    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        tx: SyncSender<Vec<i16>>,
        failed: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        let channels = config.channels as usize;
        let mut resampler = Resampler::new(config.sample_rate.0);
        let overrun = failed.clone();
        Ok(device.build_input_stream(
            config,
            move |data: &[T], _| {
                let mono: Vec<_> = data
                    .chunks_exact(channels)
                    .map(|f| f.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32)
                    .collect();
                if tx.try_send(resampler.push(&mono)).is_err() {
                    *overrun.lock().unwrap() =
                        Some("Пропущен фрагмент записи. Повторите шаг калибровки.".into());
                }
            },
            move |e| *failed.lock().unwrap() = Some(format!("Ошибка микрофона: {e}")),
            None,
        )?)
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &config, tx, failed.clone())?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &config, tx, failed.clone())?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &config, tx, failed.clone())?,
        _ => bail!("Неподдерживаемый формат микрофона"),
    };
    stream.play()?;
    let start = Instant::now();
    let mut result = Vec::new();
    while start.elapsed() < Duration::from_secs(seconds) {
        if cancelled() {
            bail!("Калибровка отменена");
        }
        if let Ok(samples) = rx.recv_timeout(Duration::from_millis(50)) {
            result.extend(samples);
        }
    }
    drop(stream);
    if let Some(error) = failed.lock().unwrap().take() {
        bail!("{error}");
    }
    if result.is_empty() {
        bail!("Микрофон не передал звук");
    }
    Ok(result)
}

pub fn calibrate(
    state: Arc<Shared>,
    settings: Settings,
    expected: String,
    seconds: u64,
) -> Result<crate::calibration::Report> {
    settings.validate()?;
    if !(2..=8).contains(&seconds) || expected.chars().count() > 200 {
        bail!("Шаг калибровки: 2–8 секунд, фраза до 200 символов");
    }
    if state
        .busy
        .compare_exchange(false, true, SeqCst, SeqCst)
        .is_err()
    {
        bail!("Дождитесь завершения текущей команды");
    }
    struct Guard(Arc<Shared>);
    impl Drop for Guard {
        fn drop(&mut self) {
            self.0.calibration_capture.store(false, SeqCst);
            self.0.calibrating.store(false, SeqCst);
            self.0.busy.store(false, SeqCst);
        }
    }
    let _guard = Guard(state.clone());
    state.calibrating.store(true, SeqCst);
    state.epoch.fetch_add(1, SeqCst);
    state.activate.store(false, SeqCst);
    state.dictating.store(false, SeqCst);
    let epoch = state.epoch.load(SeqCst);
    for _ in 0..100 {
        if !state.capture_open.load(SeqCst) {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    if state.capture_open.load(SeqCst) {
        bail!("Фоновый микрофон не освободился. Повторите калибровку.");
    }
    state.calibration_capture.store(true, SeqCst);
    let samples = capture(&settings.microphone, seconds, || state.cancelled(epoch))?;
    state.calibration_capture.store(false, SeqCst);
    if expected.is_empty() {
        return Ok(crate::calibration::analyze(
            &samples,
            &expected,
            String::new(),
            String::new(),
        ));
    }
    let resolved = crate::config::resolve_speech(&settings, &state.resources.read().unwrap());
    let scaled: Vec<_> = samples
        .iter()
        .map(|s| (*s as f32 * settings.speech_gain).clamp(-32768.0, 32767.0) as i16)
        .collect();
    let mut vosk = Vosk::load(
        Path::new(&resolved.vosk_library),
        Path::new(&resolved.vosk_model),
        16000.0,
    )?;
    let words = crate::activation::all_wake_words(&settings);
    let is_wake = words
        .iter()
        .any(|w| crate::commands::normalize(w) == crate::commands::normalize(&expected));
    if is_wake {
        vosk.configure_wake(&words, Path::new(&resolved.vosk_model))?;
    }
    let mut transcripts = Vec::new();
    for chunk in scaled.chunks(1600) {
        if state.cancelled(epoch) {
            bail!("Калибровка отменена");
        }
        if is_wake {
            vosk.feed_wake(chunk)?;
        }
        if let Some(text) = vosk.accept(chunk)? {
            transcripts.push(text);
        }
    }
    transcripts.push(vosk.finish()?);
    let text = transcripts.join(" ").trim().to_string();
    let detected = is_wake
        && (strip_wake(&text, &words).is_some()
            || confirm_restricted_wake(&text, &vosk.finish_wake()?, &words, &settings).is_some());
    let refined = if !is_wake && settings.speech_mode == SpeechMode::Whisper {
        crate::speech::whisper_configured(&resolved, &scaled, || state.cancelled(epoch))?
    } else {
        String::new()
    };
    let mut report = crate::calibration::analyze(&samples, &expected, text, refined);
    report.wake_detected = detected;
    if is_wake && detected {
        report.matches = true;
        report.wake_candidate = None;
        report.message = "Обращение подтверждено. Вася активируется после окончания фразы.".into();
    }
    Ok(report)
}

/// Short, explicit microphone check; no recognizer or command execution runs here.
pub fn measure_input(name: &str) -> Result<f32> {
    use std::sync::atomic::AtomicU32;
    let host = cpal::default_host();
    let device = if name.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|d| d.name().ok().as_deref() == Some(name))
    }
    .context("Микрофон не найден")?;
    let config = device.default_input_config()?;
    let peak = Arc::new(AtomicU32::new(0));
    let failed = Arc::new(std::sync::Mutex::new(None));
    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        peak: Arc<AtomicU32>,
        failed: Arc<std::sync::Mutex<Option<String>>>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample,
        f32: cpal::FromSample<T>,
    {
        Ok(device.build_input_stream(
            config,
            move |samples: &[T], _| {
                let level = samples
                    .iter()
                    .map(|s| s.to_sample::<f32>().abs())
                    .fold(0.0f32, f32::max);
                peak.fetch_max(level.to_bits(), SeqCst);
            },
            move |error| {
                *failed.lock().unwrap() = Some(error.to_string());
            },
            Some(Duration::from_secs(3)),
        )?)
    }
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => {
            build::<f32>(&device, &config.into(), peak.clone(), failed.clone())?
        }
        cpal::SampleFormat::I16 => {
            build::<i16>(&device, &config.into(), peak.clone(), failed.clone())?
        }
        cpal::SampleFormat::U16 => {
            build::<u16>(&device, &config.into(), peak.clone(), failed.clone())?
        }
        _ => bail!("Неподдерживаемый формат микрофона"),
    };
    stream.play()?;
    std::thread::sleep(Duration::from_secs(2));
    drop(stream);
    if let Some(error) = failed.lock().unwrap().take() {
        bail!("{error}");
    }
    Ok(f32::from_bits(peak.load(SeqCst)))
}

/// Stateful linear resampling, including phase across arbitrary callback boundaries.
struct Resampler {
    ratio: f64,
    position: f64,
    previous: f32,
    initialized: bool,
}
impl Resampler {
    fn new(rate: u32) -> Self {
        Self {
            ratio: rate as f64 / 16000.0,
            position: 0.0,
            previous: 0.0,
            initialized: false,
        }
    }
    fn push(&mut self, input: &[f32]) -> Vec<i16> {
        let mut output = Vec::new();
        for &sample in input {
            if !self.initialized {
                self.previous = sample;
                self.initialized = true;
            }
            while self.position < 1.0 {
                let value = self.previous + (sample - self.previous) * self.position as f32;
                output.push((value.clamp(-1.0, 1.0) * 32767.0) as i16);
                self.position += self.ratio;
            }
            self.position -= 1.0;
            self.previous = sample;
        }
        output
    }
}

fn stream(settings: &Settings, state: Arc<Shared>) -> Result<(cpal::Stream, Receiver<Vec<i16>>)> {
    let host = cpal::default_host();
    let device = if settings.microphone.is_empty() {
        host.default_input_device()
    } else {
        host.input_devices()?
            .find(|d| d.name().ok().as_deref() == Some(&settings.microphone))
    }
    .context("Микрофон не найден. Проверьте выбор и разрешения Windows.")?;
    let supported = device.default_input_config()?;
    let channels = supported.channels() as usize;
    let rate = supported.sample_rate().0;
    let config: cpal::StreamConfig = supported.clone().into();
    let (tx, rx) = mpsc::sync_channel(48);
    fn build<T>(
        device: &cpal::Device,
        config: &cpal::StreamConfig,
        channels: usize,
        rate: u32,
        gain: f32,
        tx: SyncSender<Vec<i16>>,
        state: Arc<Shared>,
    ) -> Result<cpal::Stream>
    where
        T: cpal::SizedSample + cpal::Sample,
        f32: cpal::FromSample<T>,
    {
        let mut resampler = Resampler::new(rate);
        let mut level_time = Instant::now();
        let errors = state.clone();
        Ok(device.build_input_stream(
            config,
            move |data: &[T], _| {
                if state.muted.load(SeqCst) || state.busy.load(SeqCst) || state.stop.load(SeqCst) {
                    return;
                }
                let mono: Vec<f32> = data
                    .chunks_exact(channels)
                    .map(|frame| {
                        (frame.iter().map(|s| s.to_sample::<f32>()).sum::<f32>() / channels as f32
                            * gain)
                            .clamp(-1.0, 1.0)
                    })
                    .collect();
                if level_time.elapsed() > Duration::from_millis(100) {
                    let rms =
                        (mono.iter().map(|s| s * s).sum::<f32>() / mono.len().max(1) as f32).sqrt();
                    state.level((rms * 5.0).min(1.0));
                    level_time = Instant::now();
                }
                let samples = resampler.push(&mono);
                if tx.try_send(samples).is_err() {
                    // A gap invalidates the current utterance instead of executing a truncated command.
                    state.epoch.fetch_add(1, SeqCst);
                }
            },
            move |error| {
                errors.emit(
                    "error",
                    format!("Ошибка микрофона: {error}. Сохраните настройки для перезапуска."),
                );
                errors.epoch.fetch_add(1, SeqCst);
            },
            Some(Duration::from_secs(3)),
        )?)
    }
    let stream = match supported.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(
            &device,
            &config,
            channels,
            rate,
            settings.speech_gain,
            tx,
            state,
        )?,
        cpal::SampleFormat::I16 => build::<i16>(
            &device,
            &config,
            channels,
            rate,
            settings.speech_gain,
            tx,
            state,
        )?,
        cpal::SampleFormat::U16 => build::<u16>(
            &device,
            &config,
            channels,
            rate,
            settings.speech_gain,
            tx,
            state,
        )?,
        other => bail!("Неподдерживаемый формат микрофона: {other}"),
    };
    stream.play()?;
    Ok((stream, rx))
}

pub fn run(state: Arc<Shared>, sender: SyncSender<Task>) {
    while !state.stop.load(SeqCst) {
        if !state.snapshot().onboarding_complete {
            state.emit("status", "Завершите первый запуск");
            while !state.stop.load(SeqCst) && !state.snapshot().onboarding_complete {
                std::thread::sleep(Duration::from_millis(100));
            }
            continue;
        }
        let version = state.config_version.load(SeqCst);
        if let Err(error) = session(&state, &sender, version) {
            state.capture_open.store(false, SeqCst);
            state.emit("status", "Требуется настройка микрофона или модели");
            state.emit("error", format!("{error:#}"));
            while !state.stop.load(SeqCst) && state.config_version.load(SeqCst) == version {
                std::thread::sleep(Duration::from_millis(100));
            }
        }
    }
    state.capture_open.store(false, SeqCst);
}

fn session(state: &Arc<Shared>, sender: &SyncSender<Task>, version: u64) -> Result<()> {
    let settings =
        crate::config::resolve_speech(&state.snapshot(), &state.resources.read().unwrap());
    state.emit("status", "Загружаю локальную модель");
    let mut vosk = Vosk::load(
        Path::new(&settings.vosk_library),
        Path::new(&settings.vosk_model),
        16000.0,
    )?;
    let wake_words = crate::activation::all_wake_words(&settings);
    if settings.llm_repair_speech {
        vosk.enable_word_confidence();
    }
    if settings.activation != Activation::Hotkey {
        vosk.configure_wake(&wake_words, Path::new(&settings.vosk_model))?;
    }
    let mut gate = Gate::new(
        settings.activation.clone(),
        wake_words.clone(),
        settings.command_timeout,
    );
    let mut input = None;
    let mut utterance = Vec::new();
    let mut epoch = state.epoch.load(SeqCst);
    let mut status = String::new();
    while !state.stop.load(SeqCst) && state.config_version.load(SeqCst) == version {
        let now = Instant::now();
        if epoch != state.epoch.load(SeqCst) {
            epoch = state.epoch.load(SeqCst);
            gate.reset();
            vosk.reset();
            utterance.clear();
            input = None;
            state.capture_open.store(false, SeqCst);
        }
        if state.activate.swap(false, SeqCst) {
            // Do not reinterpret audio buffered before the key press as a new command.
            input = None;
            state.capture_open.store(false, SeqCst);
            gate.arm(now);
            vosk.reset();
            utterance.clear();
        }
        let muted = state.muted.load(SeqCst);
        let busy = state.busy.load(SeqCst);
        let dictating = state.dictating.load(SeqCst);
        let should_capture = !muted
            && !busy
            && (settings.activation != Activation::Hotkey || gate.armed(now) || dictating);
        let label = if muted {
            "Микрофон отключён"
        } else if busy {
            "Выполняю команду"
        } else if dictating {
            "Диктовка · скажите «стоп диктовка» для завершения"
        } else if gate.armed(now) {
            "Слушаю команду…"
        } else if settings.activation == Activation::Hotkey {
            "Ожидаю горячую клавишу"
        } else {
            "Ожидаю слово активации"
        };
        if label != status {
            status = label.into();
            state.emit("status", label);
        }
        if !should_capture {
            input = None;
            state.capture_open.store(false, SeqCst);
            vosk.reset();
            utterance.clear();
            if muted || busy {
                gate.reset();
            }
            std::thread::sleep(Duration::from_millis(25));
            continue;
        }
        if input.is_none() {
            input = Some(stream(&settings, state.clone())?);
            state.capture_open.store(true, SeqCst);
        }
        let (_, receiver) = input.as_ref().unwrap();
        let samples = match receiver.recv_timeout(Duration::from_millis(40)) {
            Ok(samples) => samples,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(_) => bail!("Поток микрофона остановлен"),
        };
        if state.muted.load(SeqCst) || state.busy.load(SeqCst) || state.epoch.load(SeqCst) != epoch
        {
            continue;
        }
        utterance.extend_from_slice(&samples);
        let rms = (samples
            .iter()
            .map(|s| (*s as f32 / 32768.0).powi(2))
            .sum::<f32>()
            / samples.len().max(1) as f32)
            .sqrt();
        if !gate.armed(now) && !dictating && rms < settings.noise_floor && utterance.len() > 32000 {
            utterance.drain(..utterance.len() - 16000);
        }
        if utterance.len() > 16000 * 30 {
            utterance.clear();
            vosk.reset();
            gate.reset();
            state.emit(
                "info",
                "Фраза длиннее 30 секунд. Сделайте паузу и повторите команду.",
            );
            continue;
        }
        let voice_wake = !dictating && settings.activation != Activation::Hotkey;
        if voice_wake && !gate.armed(now) {
            vosk.feed_wake(&samples)?;
        }
        if let Some((text, confidence)) = vosk.accept_transcript(&samples)? {
            let text = if voice_wake && !gate.armed(now) && strip_wake(&text, &wake_words).is_none()
            {
                confirm_restricted_wake(&text, &vosk.finish_wake()?, &wake_words, &settings)
                    .unwrap_or(text)
            } else {
                text
            };
            let had_wake = voice_wake && strip_wake(&text, &wake_words).is_some();
            match gate.accept(&text, dictating, Instant::now()) {
                Accepted::Wake => {
                    state.target.store(state.platform.foreground(), SeqCst);
                    state.emit("activated", "Слушаю команду…");
                    state.emit("status", "Слушаю команду…");
                }
                Accepted::Command(body) => {
                    let uncertain = confidence
                        .iter()
                        .rev()
                        .take(body.split_whitespace().count())
                        .any(|&word| word < 0.72);
                    if had_wake {
                        state.target.store(state.platform.foreground(), SeqCst);
                        state.emit("activated", "Слушаю команду…");
                    }
                    let target = state.target.load(SeqCst);
                    if state
                        .busy
                        .compare_exchange(false, true, SeqCst, SeqCst)
                        .is_ok()
                    {
                        let samples = (settings.speech_mode == SpeechMode::Whisper)
                            .then(|| utterance.clone());
                        if sender
                            .try_send(Task::Text {
                                text: body,
                                target,
                                epoch,
                                preview: false,
                                samples,
                                had_wake,
                                dictating,
                                uncertain,
                            })
                            .is_err()
                        {
                            state.busy.store(false, SeqCst);
                        }
                    }
                }
                Accepted::Ignore => {}
            }
            utterance.clear();
            vosk.reset_wake();
        }
    }
    drop(input);
    state.capture_open.store(false, SeqCst);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn resampling_preserves_chunk_boundaries() {
        let input: Vec<_> = (0..4800).map(|i| (i as f32 * 0.03).sin()).collect();
        let mut whole = Resampler::new(48000);
        let expected = whole.push(&input);
        let mut chunks = Resampler::new(48000);
        let actual: Vec<_> = input
            .chunks(137)
            .flat_map(|chunk| chunks.push(chunk))
            .collect();
        assert_eq!(expected, actual);
        assert_eq!(expected.len(), 1600);
    }
}
