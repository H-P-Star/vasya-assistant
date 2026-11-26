use crate::{
    audio,
    config::{self, Activation, Settings},
    runtime::{Event, Runtime, Task},
};
use serde::Serialize;
use std::{
    sync::{atomic::Ordering::SeqCst, Arc, Mutex},
    time::Duration,
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, State,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

#[derive(Default)]
struct StartupErrors(Mutex<Vec<String>>);

#[derive(Serialize)]
struct Bootstrap {
    speech_ready: bool,
    settings: Settings,
    muted: bool,
    busy: bool,
    capture_open: bool,
    data_directory: String,
    errors: Vec<String>,
    status: String,
    last_error: Option<String>,
    last_result: String,
    llm_key_set: bool,
    fish_key_set: bool,
}

#[tauri::command]
fn get_state(runtime: State<'_, Runtime>, errors: State<'_, StartupErrors>) -> Bootstrap {
    Bootstrap {
        speech_ready: {
            let s = config::resolve_speech(
                &runtime.shared.snapshot(),
                &runtime.shared.resources.read().unwrap(),
            );
            std::path::Path::new(&s.vosk_library).is_file()
                && std::path::Path::new(&s.vosk_model)
                    .join("am/final.mdl")
                    .is_file()
                && (s.speech_mode == config::SpeechMode::Vosk
                    || (std::path::Path::new(&s.whisper_executable).is_file()
                        && std::path::Path::new(&s.whisper_model).is_file()))
        },
        settings: runtime.shared.snapshot(),
        muted: runtime.shared.muted.load(SeqCst),
        busy: runtime.shared.busy.load(SeqCst),
        capture_open: runtime.shared.capture_open.load(SeqCst)
            || runtime.shared.calibration_capture.load(SeqCst),
        data_directory: config::data_dir().to_string_lossy().into(),
        errors: errors.0.lock().unwrap().clone(),
        status: runtime.shared.last_status.read().unwrap().clone(),
        last_error: runtime.shared.last_error.read().unwrap().clone(),
        last_result: runtime.shared.last_result.read().unwrap().clone(),
        llm_key_set: crate::credentials::present(
            runtime.shared.snapshot().llm_provider.credential_id(),
        ),
        fish_key_set: crate::credentials::present("fish_audio"),
    }
}

#[tauri::command]
async fn list_microphones() -> Result<Vec<String>, String> {
    tauri::async_runtime::spawn_blocking(audio::devices)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn test_microphone(microphone: String) -> Result<f32, String> {
    tauri::async_runtime::spawn_blocking(move || audio::measure_input(&microphone))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| {
            format!("{e:#}. Проверьте доступ к микрофону в параметрах конфиденциальности Windows.")
        })
}

#[tauri::command]
fn builtin_commands() -> Vec<crate::commands::Builtin> {
    crate::commands::builtin_catalog()
}

#[tauri::command]
fn open_resource(kind: String) -> Result<(), String> {
    let url = match kind.as_str() {
        "piper" => "https://github.com/rhasspy/piper/releases/tag/2023.11.14-2",
        "voices" => "https://huggingface.co/rhasspy/piper-voices/tree/v1.0.0/ru/ru_RU",
        "open_router" => "https://openrouter.ai/keys",
        "open_ai" => "https://platform.openai.com/api-keys",
        "fish_audio" => "https://fish.audio/app/developers/get-started/",
        "fish_voices" => "https://fish.audio/app/discovery/",
        "docs" => "https://github.com/H-P-Star/vasya-assistant#readme",
        _ => return Err("Для этого сервиса используйте сайт его поставщика".into()),
    };
    crate::browser::open_external(url).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn open_answer_link(url: String) -> Result<(), String> {
    let url = url::Url::parse(&url).map_err(|e| e.to_string())?;
    if !crate::web_search::public_url(&url) {
        return Err("Можно открыть только публичную веб-ссылку".into());
    }
    crate::browser::open_external(url.as_str()).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn save_api_key(provider: String, key: String) -> Result<(), String> {
    crate::credentials::save(&provider, &key).map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn list_voices() -> Result<Vec<crate::voice::Voice>, String> {
    tauri::async_runtime::spawn_blocking(crate::platform::voices)
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
async fn calibrate_speech(
    settings: Settings,
    expected: String,
    seconds: u64,
    app: tauri::AppHandle,
) -> Result<crate::calibration::Report, String> {
    let state = app.state::<Runtime>().shared.clone();
    tauri::async_runtime::spawn_blocking(move || {
        audio::calibrate(state, settings, expected, seconds)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn dismiss_island(app: tauri::AppHandle) {
    crate::island::dismiss(&app);
}

#[tauri::command]
fn cancel_current(runtime: State<'_, Runtime>) {
    runtime.shared.epoch.fetch_add(1, SeqCst);
    runtime.shared.activate.store(false, SeqCst);
    runtime.shared.dictating.store(false, SeqCst);
    runtime.shared.emit("deactivated", "");
}

#[tauri::command]
fn clear_conversation(runtime: State<'_, Runtime>) -> bool {
    runtime.shared.clear_conversation();
    true
}

#[tauri::command]
fn expand_island(expanded: bool, app: tauri::AppHandle) {
    crate::island::expand(&app, expanded);
}

#[tauri::command]
fn show_last_result(app: tauri::AppHandle, runtime: State<'_, Runtime>) -> Result<(), String> {
    if !runtime.shared.snapshot().island_enabled {
        return Err("Панель отключена в настройках".into());
    }
    let text = runtime.shared.last_result.read().unwrap().clone();
    if text.is_empty() {
        return Err("Пока нет сохранённого ответа".into());
    }
    runtime.shared.emit("activated", "Последний ответ");
    runtime.shared.emit("preview_result", text);
    crate::island::expand(&app, true);
    Ok(())
}

#[tauri::command]
fn show_main(app: tauri::AppHandle) {
    crate::island::hide(&app);
    show(&app);
}

#[tauri::command]
fn copy_result(app: tauri::AppHandle, runtime: State<'_, Runtime>) -> Result<(), String> {
    let owner = app
        .get_webview_window("island")
        .ok_or("Окно панели недоступно")?
        .hwnd()
        .map_err(|e| e.to_string())?
        .0 as isize;
    crate::platform::copy_text(&runtime.shared.last_result.read().unwrap(), owner)
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn test_voice(settings: Settings, runtime: State<'_, Runtime>) -> Result<(), String> {
    settings.validate().map_err(|e| e.to_string())?;
    runtime
        .submit(Task::TestVoice(Box::new(settings)))
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn restart_music_app(runtime: State<'_, Runtime>) -> Result<(), String> {
    runtime
        .submit(Task::RestartMusic)
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn finish_setup(
    microphone: String,
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<(), String> {
    let mut settings = runtime.shared.snapshot();
    let resolved = config::resolve_speech(&settings, &runtime.shared.resources.read().unwrap());
    if !std::path::Path::new(&resolved.vosk_library).is_file()
        || !std::path::Path::new(&resolved.vosk_model)
            .join("am/final.mdl")
            .is_file()
    {
        return Err("Компоненты распознавания повреждены или отсутствуют. Переустановите приложение из полного установщика.".into());
    }
    settings.microphone = microphone;
    settings.onboarding_complete = true;
    let muted = settings.start_muted;
    save_settings(settings, app, runtime.clone())?;
    runtime.shared.set_muted(muted);
    Ok(())
}

#[tauri::command]
async fn check_llm(settings: Settings) -> Result<Vec<String>, String> {
    settings.validate().map_err(|e| e.to_string())?;
    tauri::async_runtime::spawn_blocking(move || crate::generator::models(&settings))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))
}

#[tauri::command]
fn set_muted(muted: bool, runtime: State<'_, Runtime>) {
    runtime.shared.set_muted(muted);
}

#[tauri::command]
async fn activate(app: tauri::AppHandle) -> Result<(), String> {
    let shared = app.state::<Runtime>().shared.clone();
    if shared.muted.load(SeqCst) || shared.busy.load(SeqCst) || shared.dictating.load(SeqCst) {
        shared.activate();
        return Ok(());
    }
    if let Some(window) = app.get_webview_window("main") {
        if window.is_visible().map_err(|e| e.to_string())? {
            window.minimize().map_err(|e| e.to_string())?;
        }
    }
    // Let Windows restore the previous application's focus before remembering the input target.
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(Duration::from_millis(300));
        shared.activate();
    })
    .await
    .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
fn run_text(text: String, runtime: State<'_, Runtime>) -> Result<(), String> {
    if text.trim().is_empty() || text.chars().count() > 8000 {
        return Err("Введите команду до 8000 символов".into());
    }
    let settings = runtime.shared.snapshot();
    let text = crate::activation::strip_wake(&text, &settings.wake_words)
        .unwrap_or(&text)
        .to_string();
    runtime
        .submit(Task::Text {
            text,
            target: 0,
            epoch: runtime.shared.epoch.load(SeqCst),
            preview: true,
            samples: None,
            had_wake: false,
            dictating: false,
            uncertain: false,
        })
        .map_err(|e| e.to_string())
}

#[tauri::command]
fn open_music_login(runtime: State<'_, Runtime>) -> Result<(), String> {
    runtime.submit(Task::Login).map_err(|e| e.to_string())
}

#[tauri::command]
async fn insert_result(text: String, app: tauri::AppHandle) -> Result<(), String> {
    if text.is_empty() || text.chars().count() > 200_000 {
        return Err("Текст пустой или слишком длинный".into());
    }
    let state = app.state::<Runtime>();
    if state.shared.busy.load(SeqCst) {
        return Err("Дождитесь завершения команды".into());
    }
    let epoch = state.shared.epoch.load(SeqCst);
    if let Some(window) = app.get_webview_window("main") {
        window.minimize().map_err(|e| e.to_string())?;
    }
    // User has time to select any supported target field. No attempt to steal foreground focus.
    tauri::async_runtime::spawn_blocking(move || {
        std::thread::sleep(Duration::from_secs(3));
        let state = app.state::<Runtime>();
        let target = state.shared.platform.foreground();
        state
            .submit(Task::Insert {
                text,
                target,
                epoch,
            })
            .map_err(|e| e.to_string())
    })
    .await
    .map_err(|e| e.to_string())?
}

fn apply_hotkey(
    app: &tauri::AppHandle,
    previous: Option<&Settings>,
    next: &Settings,
) -> anyhow::Result<()> {
    let old = previous
        .filter(|s| s.activation != Activation::Wake)
        .and_then(|s| s.hotkey.parse::<Shortcut>().ok());
    let new = (next.activation != Activation::Wake)
        .then(|| next.hotkey.parse::<Shortcut>())
        .transpose()?;
    if old == new && new.is_none_or(|shortcut| app.global_shortcut().is_registered(shortcut)) {
        return Ok(());
    }
    if let Some(shortcut) = new {
        app.global_shortcut().register(shortcut)?;
    }
    if let Some(shortcut) = old
        .filter(|shortcut| app.global_shortcut().is_registered(*shortcut) && Some(*shortcut) != new)
    {
        if let Err(error) = app.global_shortcut().unregister(shortcut) {
            if let Some(new) = new {
                let _ = app.global_shortcut().unregister(new);
            }
            return Err(error.into());
        }
    }
    Ok(())
}

#[tauri::command]
fn save_settings(
    settings: Settings,
    app: tauri::AppHandle,
    runtime: State<'_, Runtime>,
) -> Result<(), String> {
    settings.validate().map_err(|e| e.to_string())?;
    settings
        .hotkey
        .parse::<Shortcut>()
        .map_err(|_| "Горячая клавиша: Ctrl+Alt+Space или Ctrl+Shift+V".to_string())?;
    let previous = runtime.shared.snapshot();
    apply_hotkey(&app, Some(&previous), &settings)
        .map_err(|e| format!("Не удалось назначить горячую клавишу: {e}"))?;
    if let Err(error) = config::save(&settings, &config::data_dir().join("settings.json")) {
        let _ = apply_hotkey(&app, Some(&settings), &previous);
        return Err(error.to_string());
    }
    runtime.update(settings);
    if !runtime.shared.snapshot().island_enabled {
        if let Some(window) = app.get_webview_window("island") {
            let _ = window.hide();
        }
    }
    Ok(())
}

fn show(app: &tauri::AppHandle) {
    crate::island::hide(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

fn icon() -> tauri::image::Image<'static> {
    // Code-native tray icon: violet disc and white microphone.
    let mut rgba = vec![0u8; 32 * 32 * 4];
    for y in 0i32..32 {
        for x in 0i32..32 {
            let i = ((y * 32 + x) * 4) as usize;
            let disc = (x - 16).pow(2) + (y - 16).pow(2) < 225;
            let mic = ((13..=18).contains(&x) && (7..=18).contains(&y))
                || ((10..=21).contains(&x) && (19..=21).contains(&y))
                || ((15..=16).contains(&x) && (21..=26).contains(&y))
                || ((12..=19).contains(&x) && y == 26);
            rgba[i..i + 4].copy_from_slice(if mic {
                &[255, 255, 255, 255]
            } else if disc {
                &[112, 88, 221, 255]
            } else {
                &[0, 0, 0, 0]
            });
        }
    }
    tauri::image::Image::new_owned(rgba, 32, 32)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| show(app)))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(|app, shortcut, event| {
                    if event.state() == ShortcutState::Pressed {
                        if shortcut.to_string() == "Escape"
                            && app.state::<crate::island::Island>().active.load(SeqCst)
                        {
                            crate::island::dismiss(app);
                            return;
                        }
                        if let Some(runtime) = app.try_state::<Runtime>() {
                            runtime.shared.activate();
                        }
                    }
                })
                .build(),
        )
        .manage(StartupErrors::default())
        .manage(crate::island::Island::default())
        .setup(|app| {
            let path = config::data_dir().join("settings.json");
            let settings = match config::load(&path) {
                Ok(settings) => settings,
                Err(error) => {
                    app.state::<StartupErrors>()
                        .0
                        .lock()
                        .unwrap()
                        .push(error.to_string());
                    Settings {
                        start_muted: true,
                        ..Settings::default()
                    }
                }
            };
            let handle = app.handle().clone();
            let sink = Arc::new(move |event: Event| {
                if event.kind == "status" {
                    if let Some(tray) = handle.tray_by_id("main-tray") {
                        let _ = tray.set_tooltip(Some(format!("Вася · {}", event.text)));
                    }
                }
                crate::island::update(&handle, &event);
                if event.kind == "activated" {
                    if let Some(runtime) = handle.try_state::<Runtime>() {
                        crate::platform::activation_sound(runtime.shared.clone());
                    }
                }
                let _ = handle.emit("assistant", event);
            });
            let runtime = Runtime::start_with_resources(
                settings.clone(),
                Arc::new(crate::platform::Windows),
                sink,
                app.path().resource_dir()?,
            );
            app.manage(runtime);
            if let Err(error) = apply_hotkey(app.handle(), None, &settings) {
                app.state::<StartupErrors>()
                    .0
                    .lock()
                    .unwrap()
                    .push(format!("Горячая клавиша: {error}"));
            }
            let open = MenuItem::with_id(app, "open", "Открыть Васю", true, None::<&str>)?;
            let mic = MenuItem::with_id(
                app,
                "mic",
                "Включить / отключить микрофон",
                true,
                None::<&str>,
            )?;
            let listen = MenuItem::with_id(app, "listen", "Слушать команду", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Выход", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&open, &mic, &listen, &quit])?;
            TrayIconBuilder::with_id("main-tray")
                .icon(icon())
                .tooltip("Вася · локальный помощник")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "open" => show(app),
                    "mic" => {
                        let state = app.state::<Runtime>();
                        state.shared.set_muted(!state.shared.muted.load(SeqCst));
                    }
                    "listen" => {
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            if let Err(error) = activate(app.clone()).await {
                                app.state::<Runtime>().shared.emit("error", error);
                            }
                        });
                    }
                    "quit" => {
                        app.state::<Runtime>().close();
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show(tray.app_handle());
                    }
                })
                .build(app)?;
            if settings.onboarding_complete
                && settings.start_in_tray
                && app.state::<StartupErrors>().0.lock().unwrap().is_empty()
            {
                if let Some(window) = app.get_webview_window("main") {
                    window.hide()?;
                }
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                if window.label() == "island" {
                    crate::island::dismiss(window.app_handle());
                } else {
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            list_microphones,
            set_muted,
            activate,
            run_text,
            open_music_login,
            insert_result,
            save_settings,
            check_llm,
            test_microphone,
            finish_setup,
            builtin_commands,
            save_api_key,
            list_voices,
            calibrate_speech,
            dismiss_island,
            expand_island,
            show_last_result,
            show_main,
            copy_result,
            test_voice,
            restart_music_app,
            cancel_current,
            clear_conversation,
            open_answer_link,
            open_resource
        ])
        .build(tauri::generate_context!())
        .expect("Не удалось запустить Васю")
        .run(|app, event| {
            if let tauri::RunEvent::Exit = event {
                if let Some(state) = app.try_state::<Runtime>() {
                    state.close();
                }
            }
        });
}
