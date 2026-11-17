use anyhow::{bail, Context, Result};
use libloading::Library;
use std::{
    ffi::{c_char, c_int, c_void, CStr, CString},
    path::Path,
    ptr::NonNull,
};

type NewModel = unsafe extern "C" fn(*const c_char) -> *mut c_void;
type Free = unsafe extern "C" fn(*mut c_void);
type NewRecognizer = unsafe extern "C" fn(*mut c_void, f32) -> *mut c_void;
type Accept = unsafe extern "C" fn(*mut c_void, *const i16, c_int) -> c_int;
type Text = unsafe extern "C" fn(*mut c_void) -> *const c_char;
type Reset = unsafe extern "C" fn(*mut c_void);
type NewGrammar = unsafe extern "C" fn(*mut c_void, f32, *const c_char) -> *mut c_void;
type FindWord = unsafe extern "C" fn(*mut c_void, *const c_char) -> c_int;

fn model_path_cstring(model: &Path) -> Result<CString> {
    // Tauri canonicalizes the executable path; Windows then adds a \\?\ prefix.
    // Rust can read that path, but Vosk's narrow stat/ifstream cannot reliably do so.
    let ordinary = dunce::canonicalize(model)
        .with_context(|| format!("Не удалось получить путь модели «{}»", model.display()))?;
    Ok(CString::new(ordinary.to_str().context(
        "Путь модели содержит неподдерживаемые символы",
    )?)?)
}

fn validate_model(model: &Path) -> Result<()> {
    use std::io::Read;
    let files = if model.join("am/final.mdl").is_file() {
        vec!["am/final.mdl", "conf/model.conf"]
    } else if model.join("final.mdl").is_file() {
        vec!["final.mdl", "mfcc.conf"]
    } else {
        bail!("В папке модели «{}» не найден am/final.mdl. Выберите папку vosk-model-small-ru-0.22, внутри которой находятся am, conf и graph.", model.display());
    };
    for relative in files {
        let path = model.join(relative);
        let mut file = std::fs::File::open(&path)
            .with_context(|| format!("Не удаётся прочитать файл модели «{}»", path.display()))?;
        if file.read(&mut [0u8; 1])? == 0 {
            bail!(
                "Файл модели пуст: {}. Переустановите полный комплект.",
                path.display()
            );
        }
    }
    Ok(())
}

pub struct Vosk {
    // Kept alive until both opaque handles have been freed in Drop.
    _library: Library,
    model: NonNull<c_void>,
    recognizer: NonNull<c_void>,
    free_model: Free,
    free_recognizer: Free,
    accept: Accept,
    result: Text,
    partial_result: Text,
    final_result: Text,
    wake_recognizer: Option<NonNull<c_void>>,
    reset: Reset,
}

impl Vosk {
    pub fn load(library: &Path, model: &Path, sample_rate: f32) -> Result<Self> {
        if !library.is_file() {
            bail!(
                "Не найдены компоненты распознавания. Переустановите Васю из полного установщика."
            );
        }
        if !model.is_dir() {
            bail!("Не найдена папка модели Vosk. Укажите распакованную модель в настройках.");
        }
        validate_model(model)?;
        let model_path = model_path_cstring(model)?;
        // SAFETY: User explicitly configures a trusted native Vosk library. All symbol signatures
        // match vosk_api.h; the library outlives its handles, which stay on their creating thread.
        unsafe {
            #[cfg(windows)]
            let lib: Library =
                libloading::os::windows::Library::load_with_flags(library, 0x100 | 0x1000)
                    .context("Не удалось загрузить Vosk и зависимые DLL из её папки")?
                    .into();
            #[cfg(not(windows))]
            let lib = Library::new(library)?;
            let new_model: NewModel = *lib.get(b"vosk_model_new\0")?;
            let free_model: Free = *lib.get(b"vosk_model_free\0")?;
            let new_recognizer: NewRecognizer = *lib.get(b"vosk_recognizer_new\0")?;
            let free_recognizer: Free = *lib.get(b"vosk_recognizer_free\0")?;
            let accept: Accept = *lib.get(b"vosk_recognizer_accept_waveform_s\0")?;
            let result: Text = *lib.get(b"vosk_recognizer_result\0")?;
            let partial_result: Text = *lib.get(b"vosk_recognizer_partial_result\0")?;
            let final_result: Text = *lib.get(b"vosk_recognizer_final_result\0")?;
            let reset: Reset = *lib.get(b"vosk_recognizer_reset\0")?;
            let set_log: unsafe extern "C" fn(c_int) = *lib.get(b"vosk_set_log_level\0")?;
            set_log(-1);
            let handle = NonNull::new(new_model(model_path.as_ptr()));
            let model = handle.with_context(|| format!("Vosk не смогла открыть модель «{}». Основные файлы доступны, но библиотека не смогла их загрузить. Проверьте полный комплект модели и её совместимость с Vosk. Микрофон здесь ещё не используется.", model.display()))?;
            let Some(recognizer) = NonNull::new(new_recognizer(model.as_ptr(), sample_rate)) else {
                free_model(model.as_ptr());
                bail!("Vosk не смогла создать распознаватель");
            };
            Ok(Self {
                _library: lib,
                model,
                recognizer,
                free_model,
                free_recognizer,
                accept,
                result,
                partial_result,
                final_result,
                wake_recognizer: None,
                reset,
            })
        }
    }

    pub fn accept(&mut self, samples: &[i16]) -> Result<Option<String>> {
        // SAFETY: Both buffers and recognizer are valid for this synchronous call.
        let status = unsafe {
            (self.accept)(
                self.recognizer.as_ptr(),
                samples.as_ptr(),
                samples.len() as c_int,
            )
        };
        if status < 0 {
            bail!("Ошибка обработки звука Vosk");
        }
        if status == 0 {
            return Ok(None);
        }
        let ptr = unsafe { (self.result)(self.recognizer.as_ptr()) };
        if ptr.is_null() {
            bail!("Vosk вернула пустой указатель");
        }
        let json: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(ptr) }.to_bytes())?;
        Ok(Some(json["text"].as_str().unwrap_or_default().into()))
    }

    pub fn reset(&mut self) {
        unsafe { (self.reset)(self.recognizer.as_ptr()) };
        self.reset_wake();
    }

    pub fn configure_wake(&mut self, words: &[String], model_path: &Path) -> Result<()> {
        // Grammar adaptation requires the dynamic graph shipped by the small model.
        if !model_path.join("graph/HCLr.fst").is_file() {
            return Ok(());
        }
        unsafe {
            let find: FindWord = *self._library.get(b"vosk_model_find_word\0")?;
            let create: NewGrammar = *self._library.get(b"vosk_recognizer_new_grm\0")?;
            let mut known: Vec<_> = words
                .iter()
                .filter(|word| {
                    word.split_whitespace().all(|w| {
                        CString::new(w).is_ok_and(|w| find(self.model.as_ptr(), w.as_ptr()) >= 0)
                    })
                })
                .cloned()
                .collect();
            if known.is_empty() {
                return Ok(());
            }
            known.push("[unk]".into());
            let grammar = CString::new(serde_json::to_string(&known)?)?;
            self.wake_recognizer =
                NonNull::new(create(self.model.as_ptr(), 16000.0, grammar.as_ptr()));
        }
        Ok(())
    }

    pub fn reset_wake(&mut self) {
        if let Some(handle) = self.wake_recognizer {
            unsafe { (self.reset)(handle.as_ptr()) };
        }
    }

    pub fn wake(&mut self, samples: &[i16]) -> Result<String> {
        let Some(handle) = self.wake_recognizer else {
            return Ok(String::new());
        };
        unsafe {
            let complete = (self.accept)(handle.as_ptr(), samples.as_ptr(), samples.len() as c_int);
            if complete < 0 {
                bail!("Ошибка детектора активации Vosk");
            }
            let ptr = if complete == 0 {
                (self.partial_result)(handle.as_ptr())
            } else {
                (self.result)(handle.as_ptr())
            };
            if ptr.is_null() {
                return Ok(String::new());
            }
            let json: serde_json::Value = serde_json::from_slice(CStr::from_ptr(ptr).to_bytes())?;
            Ok(json[if complete == 0 { "partial" } else { "text" }]
                .as_str()
                .unwrap_or_default()
                .into())
        }
    }

    pub fn finish(&mut self) -> Result<String> {
        let ptr = unsafe { (self.final_result)(self.recognizer.as_ptr()) };
        if ptr.is_null() {
            bail!("Vosk вернула пустой финальный результат");
        }
        let json: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(ptr) }.to_bytes())?;
        Ok(json["text"].as_str().unwrap_or_default().into())
    }

    pub fn partial(&mut self) -> Result<String> {
        // SAFETY: Vosk owns the NUL-terminated result until the next recognizer call.
        let ptr = unsafe { (self.partial_result)(self.recognizer.as_ptr()) };
        if ptr.is_null() {
            bail!("Vosk вернула пустой частичный результат");
        }
        let json: serde_json::Value =
            serde_json::from_slice(unsafe { CStr::from_ptr(ptr) }.to_bytes())?;
        Ok(json["partial"].as_str().unwrap_or_default().into())
    }
}

impl Drop for Vosk {
    fn drop(&mut self) {
        unsafe {
            if let Some(handle) = self.wake_recognizer {
                (self.free_recognizer)(handle.as_ptr());
            }
            (self.free_recognizer)(self.recognizer.as_ptr());
            (self.free_model)(self.model.as_ptr());
        }
    }
}

pub fn write_wav(path: &Path, samples: &[i16]) -> Result<()> {
    use std::io::Write;
    let mut file = std::fs::File::create(path)?;
    let size = (samples.len() * 2) as u32;
    file.write_all(b"RIFF")?;
    file.write_all(&(36 + size).to_le_bytes())?;
    file.write_all(b"WAVEfmt ")?;
    file.write_all(&16u32.to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?; // PCM
    file.write_all(&1u16.to_le_bytes())?; // Mono
    file.write_all(&16000u32.to_le_bytes())?;
    file.write_all(&32000u32.to_le_bytes())?;
    file.write_all(&2u16.to_le_bytes())?;
    file.write_all(&16u16.to_le_bytes())?;
    file.write_all(b"data")?;
    file.write_all(&size.to_le_bytes())?;
    for sample in samples {
        file.write_all(&sample.to_le_bytes())?;
    }
    Ok(())
}

pub fn whisper(
    executable: &Path,
    model: &Path,
    samples: &[i16],
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    whisper_options(executable, model, samples, "auto", "", cancelled)
}

pub fn whisper_configured(
    settings: &crate::config::Settings,
    samples: &[i16],
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    let names = settings
        .speech_corrections
        .iter()
        .map(|c| c.replacement.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    let hint: String = format!("Spotify, YouTube, IPv6. {names} {}", settings.speech_hint)
        .chars()
        .take(2500)
        .collect();
    whisper_options(
        Path::new(&settings.whisper_executable),
        Path::new(&settings.whisper_model),
        samples,
        &settings.speech_language,
        &hint,
        cancelled,
    )
}

fn whisper_options(
    executable: &Path,
    model: &Path,
    samples: &[i16],
    language: &str,
    hint: &str,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    if !executable.is_file() || !model.is_file() {
        bail!("Укажите whisper-cli.exe и локальную ggml-модель Whisper");
    }
    let dir = tempfile::tempdir()?;
    let wav = dir.path().join("speech.wav");
    let output = dir.path().join("result");
    write_wav(&wav, samples)?;
    let mut command = Command::new(executable);
    // Short commands need less encoder context than the model's full 30 s window.
    let context = ((samples.len() / 16000 + 3) * 50).clamp(512, 1500);
    command
        .args(["-l", language, "-nt", "-np", "-otxt", "-of"])
        .arg(&output)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    command
        .args(["-bs", "5", "-bo", "1", "-mc", "224", "-ac"])
        .arg(context.to_string());
    if !hint.trim().is_empty() {
        command.arg("--prompt").arg(hint);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .context("Не удалось запустить whisper-cli")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            if !status.success() {
                bail!("Whisper завершился с ошибкой. Проверьте модель и DLL whisper.cpp.");
            }
            break;
        }
        if cancelled() || start.elapsed() > Duration::from_secs(90) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("Распознавание Whisper отменено или превышено время ожидания");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    Ok(std::fs::read_to_string(output.with_extension("txt"))?
        .trim()
        .into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn wrong_model_root_reports_actual_directory() {
        let dir = tempfile::tempdir().unwrap();
        let error = validate_model(dir.path()).unwrap_err().to_string();
        assert!(error.contains("am/final.mdl"));
        assert!(error.contains(&dir.path().display().to_string()));
    }

    #[test]
    fn missing_or_empty_config_is_reported_before_native_load() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("am")).unwrap();
        std::fs::write(dir.path().join("am/final.mdl"), [1]).unwrap();
        assert!(validate_model(dir.path())
            .unwrap_err()
            .to_string()
            .contains("conf/model.conf"));
        std::fs::create_dir_all(dir.path().join("conf")).unwrap();
        std::fs::write(dir.path().join("conf/model.conf"), []).unwrap();
        assert!(validate_model(dir.path())
            .unwrap_err()
            .to_string()
            .contains("пуст"));
    }

    #[cfg(windows)]
    #[test]
    fn tauri_style_verbatim_model_path_is_converted_for_vosk() {
        let dir = tempfile::tempdir().unwrap();
        let verbatim = std::fs::canonicalize(dir.path()).unwrap();
        assert!(verbatim.to_string_lossy().starts_with(r"\\?\"));
        let encoded = model_path_cstring(&verbatim).unwrap();
        let text = encoded.to_str().unwrap();
        assert!(!text.starts_with(r"\\?\"));
        assert!(Path::new(text).is_dir());
    }

    #[test]
    fn wav_has_valid_pcm_header() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.wav");
        write_wav(&path, &[0, 32767, -32768]).unwrap();
        let bytes = std::fs::read(path).unwrap();
        assert_eq!(&bytes[..4], b"RIFF");
        assert_eq!(bytes.len(), 50);
        assert_eq!(u32::from_le_bytes(bytes[40..44].try_into().unwrap()), 6);
        assert_eq!(&bytes[44..], &[0, 0, 255, 127, 0, 128]);
    }
}
