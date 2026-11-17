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

pub struct Vosk {
    // Kept alive until both opaque handles have been freed in Drop.
    _library: Library,
    model: NonNull<c_void>,
    recognizer: NonNull<c_void>,
    free_model: Free,
    free_recognizer: Free,
    accept: Accept,
    result: Text,
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
        let model_path = CString::new(model.to_string_lossy().as_bytes())?;
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
            let reset: Reset = *lib.get(b"vosk_recognizer_reset\0")?;
            let set_log: unsafe extern "C" fn(c_int) = *lib.get(b"vosk_set_log_level\0")?;
            set_log(-1);
            let model = NonNull::new(new_model(model_path.as_ptr()))
                .context("Vosk не смогла открыть модель")?;
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
    }
}

impl Drop for Vosk {
    fn drop(&mut self) {
        unsafe {
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
    command
        .args(["-l", "ru", "-nt", "-np", "-otxt", "-of"])
        .arg(&output)
        .arg("-m")
        .arg(model)
        .arg("-f")
        .arg(wav)
        .stdout(Stdio::null())
        .stderr(Stdio::null());
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
