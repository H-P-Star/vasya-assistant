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
            Speech::{
                ISpObjectToken, ISpObjectTokenCategory, ISpVoice, SpObjectToken,
                SpObjectTokenCategory, SpVoice, SPCAT_VOICES, SPF_ASYNC, SPF_IS_NOT_XML,
                SPF_PURGEBEFORESPEAK,
            },
        },
        System::{
            Com::{
                CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
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

pub fn copy_text(text: &str, owner: isize) -> Result<()> {
    use windows::Win32::{
        Foundation::{GlobalFree, HANDLE},
        System::{
            DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData},
            Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE},
        },
    };
    let chars: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
    unsafe {
        OpenClipboard(Some(HWND(owner as *mut std::ffi::c_void)))?;
        struct Clipboard;
        impl Drop for Clipboard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _guard = Clipboard;
        let memory = GlobalAlloc(GMEM_MOVEABLE, chars.len() * 2)?;
        let data = GlobalLock(memory);
        if data.is_null() {
            let _ = GlobalFree(Some(memory));
            bail!("Не удалось выделить память буфера обмена");
        }
        std::ptr::copy_nonoverlapping(chars.as_ptr(), data.cast(), chars.len());
        let _ = GlobalUnlock(memory);
        if let Err(error) =
            EmptyClipboard().and_then(|_| SetClipboardData(13, Some(HANDLE(memory.0))).map(|_| ()))
        {
            let _ = GlobalFree(Some(memory));
            return Err(error.into());
        }
    }
    Ok(())
}

fn take_com_string(ptr: windows::core::PWSTR) -> Result<String> {
    unsafe {
        let value = ptr.to_string();
        CoTaskMemFree(Some(ptr.0.cast()));
        Ok(value?)
    }
}

pub fn voices() -> Result<Vec<crate::voice::Voice>> {
    let _com = Com::init()?;
    unsafe {
        let category: ISpObjectTokenCategory =
            CoCreateInstance(&SpObjectTokenCategory, None, CLSCTX_ALL)?;
        category.SetId(SPCAT_VOICES, false)?;
        let tokens = category.EnumTokens(PCWSTR::null(), PCWSTR::null())?;
        let mut count = 0;
        tokens.GetCount(&mut count)?;
        let mut voices = Vec::new();
        for i in 0..count {
            let token = tokens.Item(i)?;
            voices.push(crate::voice::Voice {
                id: take_com_string(token.GetId()?)?,
                name: take_com_string(token.GetStringValue(PCWSTR::null())?)?,
            });
        }
        Ok(voices)
    }
}

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
    fn stop_app(&self, path: &str) -> Result<()> {
        use windows::core::PWSTR;
        use windows::Win32::{
            Foundation::{CloseHandle, MAX_PATH},
            System::{
                Diagnostics::ToolHelp::{
                    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W,
                    TH32CS_SNAPPROCESS,
                },
                Threading::{
                    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, PROCESS_NAME_WIN32,
                    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE,
                },
            },
        };
        let expected = dunce::canonicalize(path)?.to_string_lossy().to_lowercase();
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0)?;
            struct Handle(windows::Win32::Foundation::HANDLE);
            impl Drop for Handle {
                fn drop(&mut self) {
                    unsafe {
                        let _ = CloseHandle(self.0);
                    }
                }
            }
            let _snapshot = Handle(snapshot);
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut next = Process32FirstW(snapshot, &mut entry);
            while next.is_ok() {
                if entry.th32ProcessID != std::process::id() {
                    if let Ok(process) = OpenProcess(
                        PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_TERMINATE,
                        false,
                        entry.th32ProcessID,
                    ) {
                        let _process = Handle(process);
                        let mut buffer = vec![0u16; MAX_PATH as usize * 8];
                        let mut size = buffer.len() as u32;
                        if QueryFullProcessImageNameW(
                            process,
                            PROCESS_NAME_WIN32,
                            PWSTR(buffer.as_mut_ptr()),
                            &mut size,
                        )
                        .is_ok()
                        {
                            let actual = String::from_utf16_lossy(&buffer[..size as usize]);
                            if actual.to_lowercase() == expected {
                                TerminateProcess(process, 0)?;
                            }
                        }
                    }
                }
                next = Process32NextW(snapshot, &mut entry);
            }
        }
        Ok(())
    }
    fn speak_configured(
        &self,
        text: &str,
        settings: &crate::config::Settings,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<()> {
        use windows::Win32::{
            Foundation::WAIT_OBJECT_0,
            Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT, SND_SYNC},
            System::Threading::WaitForSingleObject,
        };
        if settings.tts_volume == 0 || cancelled() {
            return Ok(());
        }
        if settings.tts_engine == "piper" {
            let mut bytes = crate::voice::synthesize(settings, text, cancelled)?;
            let duration = crate::voice::scale_wav(&mut bytes, 100)?;
            if cancelled() {
                return Ok(());
            }
            unsafe {
                if !PlaySoundW(
                    PCWSTR(bytes.as_ptr().cast()),
                    None,
                    SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
                )
                .as_bool()
                {
                    bail!("Windows не смогла воспроизвести голос Piper");
                }
            }
            let start = std::time::Instant::now();
            while start.elapsed() < duration && !cancelled() {
                std::thread::sleep(std::time::Duration::from_millis(40));
            }
            unsafe {
                let _ = PlaySoundW(PCWSTR::null(), None, SND_SYNC);
            }
            return Ok(());
        }
        let _com = Com::init()?;
        unsafe {
            let voice: ISpVoice = CoCreateInstance(&SpVoice, None, CLSCTX_ALL)?;
            if !settings.tts_voice.is_empty() {
                let token: ISpObjectToken = CoCreateInstance(&SpObjectToken, None, CLSCTX_ALL)?;
                token.SetId(PCWSTR::null(), &HSTRING::from(&settings.tts_voice), false)?;
                voice.SetVoice(&token)?;
            }
            voice.SetVolume(settings.tts_volume)?;
            voice.SetRate(settings.tts_rate)?;
            let text: Vec<u16> = text.encode_utf16().chain(Some(0)).collect();
            voice.Speak(
                PCWSTR(text.as_ptr()),
                (SPF_ASYNC.0 | SPF_IS_NOT_XML.0) as u32,
                None,
            )?;
            let start = std::time::Instant::now();
            loop {
                if WaitForSingleObject(voice.SpeakCompleteEvent(), 40) == WAIT_OBJECT_0 {
                    break;
                }
                if cancelled() || start.elapsed() > std::time::Duration::from_secs(180) {
                    voice.Speak(
                        PCWSTR::null(),
                        (SPF_PURGEBEFORESPEAK.0 | SPF_ASYNC.0) as u32,
                        None,
                    )?;
                    break;
                }
            }
        }
        Ok(())
    }
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
