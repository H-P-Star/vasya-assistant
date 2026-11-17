use anyhow::{bail, Result};
use windows::{
    core::{HSTRING, PCWSTR},
    Media::Control::GlobalSystemMediaTransportControlsSessionManager,
    Win32::{
        Foundation::HWND,
        Media::{
            Audio::{
                eConsole, eRender, Endpoints::IAudioEndpointVolume, IMMDeviceEnumerator,
                MMDeviceEnumerator,
            },
            Speech::{ISpVoice, SpVoice},
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
                COINIT_APARTMENTTHREADED,
            },
            WinRT::{RoInitialize, RoUninitialize, RO_INIT_MULTITHREADED},
        },
        UI::{
            Input::KeyboardAndMouse::{
                SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_KEYUP,
                KEYEVENTF_UNICODE, VIRTUAL_KEY,
            },
            Shell::ShellExecuteW,
            WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId, SW_SHOWNORMAL},
        },
    },
};

pub struct Windows;

pub fn open_url(url: &str) -> Result<()> {
    // SAFETY: strings remain alive for the call; callers validate web URLs or music protocols.
    let result = unsafe {
        ShellExecuteW(
            None,
            &HSTRING::from("open"),
            &HSTRING::from(url),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize <= 32 {
        bail!("Windows не удалось открыть браузер по умолчанию");
    }
    Ok(())
}

pub fn open_music_uri(uri: &str) -> Result<()> {
    let parsed = url::Url::parse(uri)?;
    if !["yandexmusic", "spotify"].contains(&parsed.scheme()) {
        bail!("Неизвестный протокол музыкального приложения");
    }
    open_url(uri).map_err(|_| anyhow::anyhow!("Музыкальное приложение не установлено или его протокол не зарегистрирован. Укажите путь к .exe в настройках."))
}

fn insert_text(text: &str, target: isize) -> Result<()> {
    let window = HWND(target as *mut std::ffi::c_void);
    let mut pid = 0;
    unsafe {
        GetWindowThreadProcessId(window, Some(&mut pid));
    }
    if target == 0 || unsafe { GetForegroundWindow() } != window || pid == std::process::id() {
        bail!("Активное окно изменилось. Текст сохранён в результате; используйте кнопку вставки.");
    }
    let mut events = Vec::new();
    for code in text.replace("\r\n", "\n").encode_utf16() {
        // Insert characters, including line feeds, without synthesizing Enter or Tab shortcuts.
        for flags in [KEYEVENTF_UNICODE, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP] {
            events.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: code,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }
    }
    for chunk in events.chunks(128) {
        if unsafe { GetForegroundWindow() } != window {
            bail!("Окно изменилось во время ввода. Вставка прервана; полный текст сохранён.");
        }
        if unsafe { SendInput(chunk, std::mem::size_of::<INPUT>() as i32) } != chunk.len() as u32 {
            bail!("Windows отклонила ввод. Приложения с повышенными правами требуют тех же прав у помощника.");
        }
    }
    Ok(())
}

struct Com;
impl Com {
    fn init() -> Result<Self> {
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED).ok()?;
        }
        Ok(Self)
    }
}
impl Drop for Com {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

impl crate::runtime::Platform for Windows {
    fn launch_app(&self, path: &str) -> Result<()> {
        let path = std::path::Path::new(path.trim().trim_matches('"'));
        if !path.is_absolute()
            || !path.is_file()
            || !path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("exe"))
        {
            bail!("Укажите полный путь к существующему .exe в редакторе команды");
        }
        std::process::Command::new(path).spawn()?;
        Ok(())
    }
    fn foreground(&self) -> isize {
        unsafe { GetForegroundWindow().0 as isize }
    }
    fn insert(&self, text: &str, target: isize) -> Result<()> {
        insert_text(text, target)
    }
    fn media(&self, action: &str) -> Result<()> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)?;
        }
        let result = (|| -> Result<()> {
            let manager =
                GlobalSystemMediaTransportControlsSessionManager::RequestAsync()?.get()?;
            let session = manager
                .GetCurrentSession()
                .map_err(|_| anyhow::anyhow!("Windows не обнаружила активный медиаплеер"))?;
            let accepted = match action {
                "pause" => session.TryPauseAsync()?.get()?,
                "resume" => session.TryPlayAsync()?.get()?,
                "next" => session.TrySkipNextAsync()?.get()?,
                _ => false,
            };
            if !accepted {
                bail!("Активный медиаплеер не поддерживает команду");
            }
            Ok(())
        })();
        unsafe {
            RoUninitialize();
        }
        result
    }
    fn volume(&self, value: Option<u8>, delta: f32) -> Result<()> {
        let _com = Com::init()?;
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let device = enumerator.GetDefaultAudioEndpoint(eRender, eConsole)?;
            let endpoint: IAudioEndpointVolume = device.Activate(CLSCTX_ALL, None)?;
            let level = value
                .map(|v| v as f32 / 100.0)
                .unwrap_or(endpoint.GetMasterVolumeLevelScalar()? + delta);
            endpoint.SetMasterVolumeLevelScalar(level.clamp(0.0, 1.0), std::ptr::null())?;
        }
        Ok(())
    }
    fn speak(&self, text: &str) -> Result<()> {
        let _com = Com::init()?;
        unsafe {
            let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
            // SAPI uses the Windows default voice; a Russian voice can be chosen in Windows settings.
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            voice.Speak(PCWSTR(text.as_ptr()), 0, None)?;
        }
        Ok(())
    }
}
