use crate::{
    browser::{self, BrowserMusic, MusicService},
    commands::{self, Command},
    config::Settings,
};
use anyhow::{bail, Result};
use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering::SeqCst},
    mpsc, Arc, Mutex, RwLock,
};

#[derive(Clone, Serialize)]
pub struct Event {
    pub kind: String,
    pub text: String,
    pub level: f32,
}

pub trait Platform: Send + Sync + 'static {
    fn foreground(&self) -> isize;
    fn insert(&self, text: &str, target: isize) -> Result<()>;
    fn media(&self, action: &str) -> Result<()>;
    fn volume(&self, value: Option<u8>, delta: f32) -> Result<()>;
    fn speak(&self, text: &str) -> Result<()>;
    fn speak_configured(&self, text: &str, _: &Settings, _: &dyn Fn() -> bool) -> Result<()> {
        self.speak(text)
    }
    fn stop_app(&self, _: &str) -> Result<()> {
        bail!("Перезапуск клиента доступен в Windows")
    }
    fn launch_app(&self, _: &str) -> Result<()> {
        bail!("Запуск приложений доступен только в Windows")
    }
}

pub struct Shared {
    pub resources: RwLock<std::path::PathBuf>,
    pub settings: RwLock<Settings>,
    pub muted: AtomicBool,
    pub busy: AtomicBool,
    pub dictating: AtomicBool,
    pub capture_open: AtomicBool,
    pub calibrating: AtomicBool,
    pub calibration_capture: AtomicBool,
    pub activate: AtomicBool,
    pub target: AtomicIsize,
    pub epoch: AtomicU64,
    pub config_version: AtomicU64,
    pub stop: AtomicBool,
    pub last_status: RwLock<String>,
    pub last_error: RwLock<Option<String>>,
    pub last_result: RwLock<String>,
    dialogue: Mutex<Vec<crate::generator::DialogueTurn>>,
    sink: Arc<dyn Fn(Event) + Send + Sync>,
    pub platform: Arc<dyn Platform>,
}

impl Shared {
    pub fn clear_conversation(&self) {
        let mut dialogue = self.dialogue.lock().unwrap();
        self.epoch.fetch_add(1, SeqCst);
        self.activate.store(false, SeqCst);
        dialogue.clear();
    }

    pub fn emit(&self, kind: &str, text: impl Into<String>) {
        let text = text.into();
        if kind == "status" {
            *self.last_status.write().unwrap() = text.clone();
        }
        if kind == "error" {
            *self.last_error.write().unwrap() = Some(text.clone());
        }
        if kind == "clear_error" {
            *self.last_error.write().unwrap() = None;
        }
        if kind == "result" {
            *self.last_result.write().unwrap() = text.clone();
        }
        (self.sink)(Event {
            kind: kind.into(),
            text,
            level: 0.0,
        });
    }
    pub fn level(&self, level: f32) {
        (self.sink)(Event {
            kind: "level".into(),
            text: String::new(),
            level,
        });
    }
    pub fn snapshot(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }
    pub fn cancelled(&self, epoch: u64) -> bool {
        self.stop.load(SeqCst) || self.epoch.load(SeqCst) != epoch
    }
    pub fn set_muted(&self, muted: bool) {
        let muted = muted || !self.snapshot().onboarding_complete;
        self.muted.store(muted, SeqCst);
        self.epoch.fetch_add(1, SeqCst);
        self.activate.store(false, SeqCst);
        self.dictating.store(false, SeqCst);
        self.emit("microphone", if muted { "muted" } else { "enabled" });
    }
    pub fn activate(&self) {
        if self.muted.load(SeqCst) {
            self.emit(
                "info",
                "Микрофон отключён. Включите его в трее или окне приложения.",
            );
            return;
        }
        if self.dictating.swap(false, SeqCst) {
            self.epoch.fetch_add(1, SeqCst);
            self.emit("status", "Диктовка завершена");
            return;
        }
        if self.busy.load(SeqCst) {
            self.emit("info", "Выполняю предыдущую команду");
            return;
        }
        self.target.store(self.platform.foreground(), SeqCst);
        self.activate.store(true, SeqCst);
        self.emit("activated", "Слушаю команду…");
    }
}

pub enum Task {
    Text {
        text: String,
        target: isize,
        epoch: u64,
        preview: bool,
        samples: Option<Vec<i16>>,
        had_wake: bool,
        dictating: bool,
        uncertain: bool,
    },
    Login,
    TestVoice(Box<Settings>),
    RestartMusic,
    Insert {
        text: String,
        target: isize,
        epoch: u64,
    },
}

pub struct Runtime {
    pub shared: Arc<Shared>,
    sender: mpsc::SyncSender<Task>,
    browser_process: browser::ProcessRegistry,
}

impl Runtime {
    pub fn start(
        settings: Settings,
        platform: Arc<dyn Platform>,
        sink: Arc<dyn Fn(Event) + Send + Sync>,
    ) -> Self {
        Self::start_with_resources(settings, platform, sink, crate::config::data_dir())
    }

    pub fn start_with_resources(
        settings: Settings,
        platform: Arc<dyn Platform>,
        sink: Arc<dyn Fn(Event) + Send + Sync>,
        resources: std::path::PathBuf,
    ) -> Self {
        let shared = Arc::new(Shared {
            resources: RwLock::new(resources),
            muted: AtomicBool::new(settings.start_muted || !settings.onboarding_complete),
            settings: RwLock::new(settings),
            busy: AtomicBool::new(false),
            dictating: AtomicBool::new(false),
            capture_open: AtomicBool::new(false),
            calibrating: AtomicBool::new(false),
            calibration_capture: AtomicBool::new(false),
            activate: AtomicBool::new(false),
            target: AtomicIsize::new(0),
            epoch: AtomicU64::new(0),
            config_version: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            sink,
            platform,
            last_status: RwLock::new("Запускаю помощника".into()),
            last_error: RwLock::new(None),
            last_result: RwLock::new(String::new()),
            dialogue: Mutex::new(Vec::new()),
        });
        let (sender, receiver) = mpsc::sync_channel(4);
        let browser_process = Arc::new(Mutex::new(browser::BrowserProcess::default()));
        let process = browser_process.clone();
        let state = shared.clone();
        std::thread::Builder::new()
            .name("vasya-actions".into())
            .spawn(move || {
                let mut music = BrowserMusic::with_process(process);
                while !state.stop.load(SeqCst) {
                    let task = match receiver.recv_timeout(std::time::Duration::from_millis(100)) {
                        Ok(task) => task,
                        Err(mpsc::RecvTimeoutError::Timeout) => continue,
                        Err(_) => break,
                    };
                    // Capture closes before SAPI speaks, preventing self-activation.
                    for _ in 0..40 {
                        if !state.capture_open.load(SeqCst) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(10));
                    }
                    let confirmation_epoch = state.epoch.load(SeqCst);
                    if matches!(&task, Task::Login | Task::RestartMusic) {
                        state.emit("clear_error", "");
                    }
                    match execute(&state, &mut music, task) {
                        Ok(message) => {
                            state.emit("info", &message);
                            if state.snapshot().voice_feedback
                                && message != "Ответ сохранён"
                                && message != "Проверка голоса завершена"
                                && !state.cancelled(confirmation_epoch)
                                && !state.stop.load(SeqCst)
                                && !(state.dictating.load(SeqCst) && message == "Текст вставлен")
                                && !state.capture_open.load(SeqCst)
                            {
                                // Keep acknowledgements brief; long results remain in the result panel.
                                let brief = if message.chars().count() > 130 {
                                    "Команда выполнена. Подробности в окне помощника."
                                } else {
                                    &message
                                };
                                if let Err(error) = state.platform.speak_configured(
                                    brief,
                                    &state.snapshot(),
                                    &|| state.cancelled(confirmation_epoch),
                                ) {
                                    state.emit(
                                        "error",
                                        format!("Голосовое подтверждение недоступно: {error:#}"),
                                    );
                                }
                            }
                        }
                        Err(error) => state.emit("error", format!("{error:#}")),
                    }
                    if !state.dictating.load(SeqCst) {
                        std::thread::sleep(std::time::Duration::from_millis(150));
                    }
                    state.busy.store(false, SeqCst);
                }
            })
            .expect("action thread");
        #[cfg(all(windows, not(test)))]
        {
            let state = shared.clone();
            let tx = sender.clone();
            std::thread::Builder::new()
                .name("vasya-audio".into())
                .spawn(move || crate::audio::run(state, tx))
                .expect("audio thread");
        }
        Self {
            shared,
            sender,
            browser_process,
        }
    }

    pub fn submit(&self, task: Task) -> Result<()> {
        if self
            .shared
            .busy
            .compare_exchange(false, true, SeqCst, SeqCst)
            .is_err()
        {
            bail!("Выполняю предыдущую команду. Дождитесь результата.");
        }
        if self.sender.try_send(task).is_err() {
            self.shared.busy.store(false, SeqCst);
            bail!("Очередь команд недоступна");
        }
        Ok(())
    }

    pub fn update(&self, settings: Settings) {
        self.shared.epoch.fetch_add(1, SeqCst);
        self.shared.dictating.store(false, SeqCst);
        *self.shared.last_error.write().unwrap() = None;
        let mut current = self.shared.settings.write().unwrap();
        if current.llm_provider != settings.llm_provider
            || current.llm_url != settings.llm_url
            || current.llm_model != settings.llm_model
        {
            self.shared.dialogue.lock().unwrap().clear();
        }
        *current = settings;
        self.shared.config_version.fetch_add(1, SeqCst);
    }
    pub fn close(&self) {
        self.shared.stop.store(true, SeqCst);
        self.shared.set_muted(true);
        // Owns only the dedicated browser launched by this runtime. Do not leave it behind
        // when an HTTP generation request is still winding down during application exit.
        self.browser_process.lock().unwrap().close();
    }
}

fn insert_if_current(
    state: &Shared,
    text: &str,
    target: isize,
    epoch: u64,
    preview: bool,
) -> Result<()> {
    state.emit("result", text);
    if preview {
        return Ok(());
    }
    if state.cancelled(epoch) {
        bail!("Вставка отменена. Текст сохранён в результате.");
    }
    state.platform.insert(text, target)
}

fn same_words(left: &str, right: &str) -> bool {
    let normalize = |text: &str| {
        text.to_lowercase()
            .replace('ё', "е")
            .split(|ch: char| !ch.is_alphanumeric())
            .filter(|word| !word.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    };
    normalize(left) == normalize(right)
}

fn repair_allowed(fixed: &Command, vosk: &Command, whisper: Option<&Command>) -> bool {
    use std::mem::discriminant;
    let recognized = |command: &Command| !matches!(command, Command::Unknown(_));
    if whisper.is_some_and(|command| {
        recognized(vosk)
            && recognized(command)
            && (discriminant(vosk) != discriminant(command)
                || (commands::time_sensitive(vosk) && vosk != command))
    }) {
        return false;
    }
    if matches!(fixed, Command::Insert(_) | Command::LaunchApp(_))
        || commands::time_sensitive(fixed)
    {
        return fixed == vosk || whisper == Some(fixed);
    }
    let fixed_kind = discriminant(fixed);
    let recognized_kind = [Some(vosk), whisper]
        .into_iter()
        .flatten()
        .filter(|command| recognized(command))
        .map(discriminant)
        .next();
    recognized_kind.map_or_else(
        || {
            !matches!(
                fixed,
                Command::OpenMusic(_) | Command::WebAnswer(_) | Command::Generate(_)
            )
        },
        |kind| kind == fixed_kind,
    )
}

fn present_answer(
    state: &Shared,
    settings: &Settings,
    question: &str,
    answer: &str,
    spoken: &str,
    epoch: u64,
) -> Result<String> {
    {
        let mut history = state.dialogue.lock().unwrap();
        if state.cancelled(epoch) {
            bail!("Команда отменена");
        }
        if history.len() >= 6 {
            history.remove(0);
        }
        history.push(crate::generator::DialogueTurn {
            question: question.chars().take(400).collect(),
            answer: answer.chars().take(1000).collect(),
        });
    }
    state.emit("result", answer);
    state.emit("answer", answer);
    if settings.voice_feedback && !state.capture_open.load(SeqCst) && !state.cancelled(epoch) {
        let brief: String = spoken.chars().take(1200).collect();
        state
            .platform
            .speak_configured(&brief, settings, &|| state.cancelled(epoch))?;
    }
    Ok("Ответ сохранён".into())
}

fn execute(state: &Shared, music: &mut dyn MusicServiceExt, task: Task) -> Result<String> {
    let settings =
        crate::config::resolve_speech(&state.snapshot(), &state.resources.read().unwrap());
    let (text, target, epoch, preview, dictating, repair_evidence) = match task {
        Task::RestartMusic => {
            let settings = Settings {
                music_service: "yandex".into(),
                music_mode: "desktop".into(),
                ..settings
            };
            let exe = browser::native_executable(&settings)?
                .ok_or_else(|| anyhow::anyhow!("Не найдено приложение Яндекс Музыки"))?;
            state.platform.stop_app(&exe.to_string_lossy())?;
            std::thread::sleep(std::time::Duration::from_millis(700));
            return music.reconnect_yandex(&settings);
        }
        Task::TestVoice(settings) => {
            if settings.tts_volume == 0 {
                let message =
                    "Громкость голоса установлена на 0%. Поднимите её и повторите проверку.";
                state.emit("voice_test", message);
                bail!("{message}");
            }
            let epoch = state.epoch.load(SeqCst);
            let result = state.platform.speak_configured(
                "Привет! Я Вася. Так звучит выбранный голос.",
                &settings,
                &|| state.cancelled(epoch),
            );
            if let Err(error) = result {
                state.emit("voice_test", format!("{error:#}"));
                return Err(error);
            }
            if state.cancelled(epoch) {
                state.emit("voice_test", "Проверка голоса отменена.");
                bail!("Проверка голоса отменена");
            }
            state.emit("voice_test", "Проверка голоса завершена.");
            return Ok("Проверка голоса завершена".into());
        }
        Task::Login => return music.login(&settings),
        Task::Insert {
            text,
            target,
            epoch,
        } => {
            insert_if_current(state, &text, target, epoch, false)?;
            return Ok("Текст вставлен".into());
        }
        Task::Text {
            text,
            target,
            epoch,
            preview,
            samples,
            had_wake,
            dictating,
            uncertain,
        } => {
            if state.cancelled(epoch) {
                bail!("Команда отменена");
            }
            let text = crate::calibration::correct(&text, &settings.speech_corrections);
            let original = commands::parse_with_settings(&text, dictating, &settings);
            let repair_enabled = !preview
                && !dictating
                && settings.llm_repair_speech
                && !matches!(
                    original,
                    Command::Disabled | Command::Invalid(_) | Command::Insert(_)
                );
            let refined = samples
                .filter(|_| !commands::time_sensitive(&original) || (repair_enabled && uncertain))
                .and_then(|samples| {
                    let refined = crate::speech::whisper_configured(&settings, &samples, || {
                        state.cancelled(epoch)
                    })
                    .ok()?;
                    let refined = if had_wake {
                        crate::activation::detected_body(
                            &refined,
                            &crate::activation::all_wake_words(&settings),
                        )
                        .to_owned()
                    } else {
                        refined
                    };
                    let refined =
                        crate::calibration::correct(&refined, &settings.speech_corrections);
                    if matches!(original, Command::Insert(_))
                        && !matches!(
                            commands::parse_with_settings(&refined, dictating, &settings),
                            Command::Insert(_)
                        )
                    {
                        return None;
                    }
                    (!refined.trim().is_empty()).then_some(refined)
                });
            let disagreement = refined
                .as_deref()
                .is_some_and(|other| !same_words(&text, other));
            if repair_enabled && (uncertain || disagreement) {
                let second = refined
                    .as_ref()
                    .map(|other| commands::parse_with_settings(other, false, &settings));
                if matches!(second, Some(Command::Insert(_))) {
                    return present_answer(
                        state,
                        &settings,
                        &text,
                        "Я не уверен, что расслышал просьбу о вставке текста. Повтори, пожалуйста.",
                        "Повтори, пожалуйста.",
                        epoch,
                    );
                }
                state.emit("status", "Уточняю распознавание…");
                let repaired =
                    crate::generator::repair_speech(&settings, &text, refined.as_deref(), || {
                        state.cancelled(epoch)
                    });
                if state.cancelled(epoch) {
                    bail!("Команда отменена");
                }
                match repaired {
                    Ok(Some(candidate))
                        if repair_allowed(
                            &commands::parse_with_settings(&candidate, false, &settings),
                            &original,
                            second.as_ref(),
                        ) =>
                    {
                        (
                            candidate,
                            target,
                            epoch,
                            preview,
                            dictating,
                            Some((original, second)),
                        )
                    }
                    Err(_) if !disagreement && !matches!(original, Command::Unknown(_)) => (
                        refined.unwrap_or(text),
                        target,
                        epoch,
                        preview,
                        dictating,
                        None,
                    ),
                    _ => {
                        let message =
                            "Я не уверен, что правильно расслышал. Повтори запрос, пожалуйста.";
                        return present_answer(state, &settings, &text, message, message, epoch);
                    }
                }
            } else {
                (
                    refined.unwrap_or(text),
                    target,
                    epoch,
                    preview,
                    dictating,
                    None,
                )
            }
        }
    };
    if state.cancelled(epoch) {
        bail!("Команда отменена");
    }
    state.emit("heard", &text);
    state.emit("status", "Выполняю команду");
    let mut command = commands::parse_with_settings(&text, dictating, &settings);
    if matches!(command, Command::Unknown(_)) && settings.llm_fallback {
        state.emit("status", "Обращаюсь к модели…");
        let history = state.dialogue.lock().unwrap().clone();
        command = crate::generator::resolve_with_history(&settings, &text, &history, || {
            state.cancelled(epoch)
        })?;
        if state.cancelled(epoch) {
            bail!("Команда отменена");
        }
        if let Some((original, second)) = &repair_evidence {
            if !matches!(command, Command::Answer(_))
                && !repair_allowed(&command, original, second.as_ref())
            {
                let message = "Я не уверен, что правильно расслышал. Повтори запрос, пожалуйста.";
                return present_answer(state, &settings, &text, message, message, epoch);
            }
        }
        state.emit("status", "Выполняю команду");
    }
    match command {
        Command::Search(query) => { browser::open_external(&browser::search_url(&query))?; Ok("Открыт поиск в браузере".into()) }
        Command::WebAnswer(query) => {
            state.emit("status", "Ищу в интернете…");
            let sources = crate::web_search::sources(&query, &settings.web_search_url, &|| state.cancelled(epoch))?;
            state.emit("status", "Сверяю источники…");
            let summary = match crate::generator::research_answer(&settings, &query, &sources, || state.cancelled(epoch)) {
                Ok(answer) => answer,
                Err(error) if !state.cancelled(epoch) => {
                    format!("Нашёл источники. Составить ответ не удалось. {error}")
                }
                Err(error) => return Err(error),
            };
            let answer = crate::web_search::with_links(&summary, &sources);
            present_answer(state, &settings, &text, &answer, &summary, epoch)
        }
        Command::Youtube(query) => browser::youtube(&query),
        Command::Music { query, category } => music.play_with_cancel(&settings, &query, &category,&||state.cancelled(epoch)),
        Command::OpenMusic(service) => music.login(&Settings { music_service: service, ..settings }),
        Command::Generate(prompt) => {
            let output = crate::generator::generate_with_cancel(&settings, &prompt, || state.cancelled(epoch))?;
            insert_if_current(state, &output, target, epoch, preview)?;
            Ok(if preview { "Текст готов в окне результата" } else { "Текст написан" }.into())
        }
        Command::Insert(text) => {
            insert_if_current(state, &text, target, epoch, preview)?;
            Ok(if preview { "Текст готов в окне результата" } else { "Текст вставлен" }.into())
        }
        action @ (Command::Pause | Command::Resume | Command::Next) => {
            let action = match action {
                Command::Pause => "pause", Command::Resume => "resume", _ => "next",
            };
            state.platform.media(action)?;
            Ok(match action { "pause" => "Пауза", "resume" => "Продолжаю воспроизведение", _ => "Следующий трек" }.into())
        }
        Command::Volume(value) => { state.platform.volume(Some(value), 0.0)?; Ok(format!("Громкость {value} процентов")) }
        Command::VolumeUp => { state.platform.volume(None, 0.1)?; Ok("Громче".into()) }
        Command::VolumeDown => { state.platform.volume(None, -0.1)?; Ok("Тише".into()) }
        Command::DictationStart => {
            if preview { bail!("Начните диктовку голосом или горячей клавишей в целевом приложении"); }
            state.target.store(target, SeqCst);
            state.dictating.store(true, SeqCst);
            Ok("Диктуйте. Для завершения скажите «стоп диктовка» или нажмите горячую клавишу.".into())
        }
        Command::DictationStop => { state.dictating.store(false, SeqCst); Ok("Диктовка завершена".into()) }
        Command::Mute => { state.set_muted(true); Ok("Микрофон отключён".into()) }
        Command::Cancel => { state.dictating.store(false, SeqCst); Ok("Отменено".into()) }
        Command::Answer(answer) => present_answer(state, &settings, &text, &answer, &answer, epoch),
        Command::LaunchApp(path) => { state.platform.launch_app(&path)?; Ok("Приложение открыто".into()) }
        Command::Disabled => Ok("Эта команда отключена в настройках".into()),
        Command::Invalid(reason) => bail!("{reason}"),
        Command::Unknown(_) => bail!("Команда не распознана. Примеры: «включи Кино», «найди IPv6», «напиши, что такое IPv6»."),
    }
}

pub trait MusicServiceExt: MusicService {
    fn login(&mut self, settings: &Settings) -> Result<String>;
    fn reconnect_yandex(&mut self, settings: &Settings) -> Result<String>;
}
impl MusicServiceExt for BrowserMusic {
    fn login(&mut self, settings: &Settings) -> Result<String> {
        BrowserMusic::login(self, settings)
    }
    fn reconnect_yandex(&mut self, settings: &Settings) -> Result<String> {
        BrowserMusic::reconnect_yandex(self, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Fake {
        inserted: Mutex<Vec<String>>,
        media_actions: Mutex<Vec<String>>,
    }
    impl Platform for Fake {
        fn foreground(&self) -> isize {
            1
        }
        fn insert(&self, text: &str, _: isize) -> Result<()> {
            self.inserted.lock().unwrap().push(text.into());
            Ok(())
        }
        fn media(&self, action: &str) -> Result<()> {
            self.media_actions.lock().unwrap().push(action.into());
            Ok(())
        }
        fn volume(&self, _: Option<u8>, _: f32) -> Result<()> {
            Ok(())
        }
        fn speak(&self, _: &str) -> Result<()> {
            Ok(())
        }
    }
    struct UnusedMusic;
    impl MusicService for UnusedMusic {
        fn play(&mut self, _: &Settings, _: &str, _: &crate::commands::Category) -> Result<String> {
            panic!("This intent must not launch music")
        }
    }
    impl MusicServiceExt for UnusedMusic {
        fn login(&mut self, _: &Settings) -> Result<String> {
            panic!("No login expected")
        }
        fn reconnect_yandex(&mut self, _: &Settings) -> Result<String> {
            panic!("No reconnect expected")
        }
    }
    fn text_task(text: &str) -> Task {
        Task::Text {
            text: text.into(),
            target: 1,
            epoch: 0,
            preview: false,
            samples: None,
            had_wake: false,
            dictating: false,
            uncertain: false,
        }
    }
    #[test]
    fn unknown_phrase_executes_resolved_media_action_and_answers_without_inserting() {
        use std::{
            io::{Read, Write},
            net::TcpListener,
        };
        for plan in [
            r#"{"action":"pause"}"#,
            r#"{"action":"answer","text":"IPv6 — протокол."}"#,
        ] {
            let server = TcpListener::bind("127.0.0.1:0").unwrap();
            let url = format!("http://{}", server.local_addr().unwrap());
            let response =
                serde_json::json!({"choices":[{"message":{"content":plan}}]}).to_string();
            let thread = std::thread::spawn(move || {
                let (mut stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                loop {
                    let mut buf = [0; 4096];
                    let n = stream.read(&mut buf).unwrap();
                    assert!(n > 0);
                    request.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&request);
                    if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                        let size = headers
                            .lines()
                            .find_map(|line| {
                                line.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .and_then(|s| s.trim().parse::<usize>().ok())
                            })
                            .unwrap();
                        if body.len() >= size {
                            break;
                        }
                    }
                }
                write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",response.len(),response).unwrap();
            });
            let platform = Arc::new(Fake::default());
            let events = Arc::new(Mutex::new(Vec::<Event>::new()));
            let log = events.clone();
            let runtime = Runtime::start(
                Settings {
                    llm_url: url,
                    voice_feedback: false,
                    ..Settings::default()
                },
                platform.clone(),
                Arc::new(move |e| log.lock().unwrap().push(e)),
            );
            execute(
                &runtime.shared,
                &mut UnusedMusic,
                text_task("пожалуйста сделай как я попросил"),
            )
            .unwrap();
            thread.join().unwrap();
            assert!(platform.inserted.lock().unwrap().is_empty());
            if plan.contains("pause") {
                assert_eq!(*platform.media_actions.lock().unwrap(), vec!["pause"]);
                assert!(runtime.shared.dialogue.lock().unwrap().is_empty());
            } else {
                assert!(events
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|e| e.kind == "result" && e.text == "IPv6 — протокол."));
                let dialogue = runtime.shared.dialogue.lock().unwrap().clone();
                assert_eq!(dialogue.len(), 1);
                assert_eq!(dialogue[0].question, "пожалуйста сделай как я попросил");
                assert_eq!(dialogue[0].answer, "IPv6 — протокол.");
                runtime.shared.clear_conversation();
                assert!(runtime.shared.dialogue.lock().unwrap().is_empty());
            }
            runtime.close();
        }
    }
    #[test]
    fn disabled_fallback_and_invalid_commands_do_not_contact_a_model() {
        let runtime = Runtime::start(
            Settings {
                llm_fallback: false,
                llm_url: "http://127.0.0.1:1".into(),
                ..Settings::default()
            },
            Arc::new(Fake::default()),
            Arc::new(|_| {}),
        );
        assert!(execute(
            &runtime.shared,
            &mut UnusedMusic,
            text_task("необычная просьба")
        )
        .unwrap_err()
        .to_string()
        .contains("не распознана"));
        runtime.shared.settings.write().unwrap().llm_fallback = true;
        assert!(execute(
            &runtime.shared,
            &mut UnusedMusic,
            text_task("громкость 999")
        )
        .unwrap_err()
        .to_string()
        .contains("Громкость"));
        runtime.close();
    }
    #[test]
    fn simple_media_commands_skip_whisper_and_keep_their_immediate_action() {
        let platform = Arc::new(Fake::default());
        let runtime = Runtime::start(Settings::default(), platform.clone(), Arc::new(|_| {}));
        let task = Task::Text {
            text: "пауза".into(),
            target: 1,
            epoch: 0,
            preview: false,
            samples: Some(vec![0; 1600]),
            had_wake: false,
            dictating: false,
            uncertain: false,
        };
        execute(&runtime.shared, &mut UnusedMusic, task).unwrap();
        assert_eq!(*platform.media_actions.lock().unwrap(), vec!["pause"]);
        runtime.close();
    }
    #[test]
    fn repair_allows_track_spelling_but_rejects_conflicting_volume_or_invented_insertion() {
        let settings = Settings::default();
        let parse = |text| commands::parse_with_settings(text, false, &settings);
        assert!(same_words("Что такое IPv6?", "что такое IPv6"));
        assert!(repair_allowed(
            &parse("включи трек Numb исполнителя Linkin Park"),
            &parse("включи трек намб исполнителя линкин парк"),
            Some(&parse("включи трек Numb исполнителя Linkin Park")),
        ));
        assert!(!repair_allowed(
            &Command::Volume(20),
            &Command::Volume(20),
            Some(&Command::Volume(90))
        ));
        assert!(!repair_allowed(
            &Command::Insert("текст".into()),
            &Command::Unknown("шум".into()),
            None
        ));
    }
    #[test]
    fn first_run_cannot_enable_background_capture_before_completion() {
        let runtime = Runtime::start(
            Settings::default(),
            Arc::new(Fake::default()),
            Arc::new(|_| {}),
        );
        runtime.shared.set_muted(false);
        assert!(runtime.shared.muted.load(SeqCst));
        runtime.shared.activate();
        assert!(!runtime.shared.activate.load(SeqCst));
        let settings = Settings {
            onboarding_complete: true,
            ..Settings::default()
        };
        runtime.update(settings);
        runtime.shared.set_muted(false);
        assert!(!runtime.shared.muted.load(SeqCst));
        runtime.close();
    }

    #[test]
    fn cancelled_or_preview_text_is_retained_without_insertion() {
        let platform = Arc::new(Fake::default());
        let events = Arc::new(Mutex::new(Vec::new()));
        let log = events.clone();
        let runtime = Runtime::start(
            Settings::default(),
            platform.clone(),
            Arc::new(move |event| log.lock().unwrap().push(event)),
        );
        let state = &runtime.shared;
        insert_if_current(state, "preview", 1, 0, true).unwrap();
        state.set_muted(true);
        assert!(insert_if_current(state, "cancelled", 1, 0, false).is_err());
        assert!(platform.inserted.lock().unwrap().is_empty());
        assert_eq!(
            events
                .lock()
                .unwrap()
                .iter()
                .filter(|e| e.kind == "result")
                .count(),
            2
        );
        let epoch = state.epoch.load(SeqCst);
        insert_if_current(state, "ok", 1, epoch, false).unwrap();
        assert_eq!(*platform.inserted.lock().unwrap(), vec!["ok"]);
        runtime.close();
    }
}
