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
}

pub struct Shared {
    pub resources: RwLock<std::path::PathBuf>,
    pub settings: RwLock<Settings>,
    pub muted: AtomicBool,
    pub busy: AtomicBool,
    pub dictating: AtomicBool,
    pub capture_open: AtomicBool,
    pub activate: AtomicBool,
    pub target: AtomicIsize,
    pub epoch: AtomicU64,
    pub config_version: AtomicU64,
    pub stop: AtomicBool,
    pub last_status: RwLock<String>,
    pub last_error: RwLock<Option<String>>,
    sink: Arc<dyn Fn(Event) + Send + Sync>,
    pub platform: Arc<dyn Platform>,
}

impl Shared {
    pub fn emit(&self, kind: &str, text: impl Into<String>) {
        let text = text.into();
        if kind == "status" {
            *self.last_status.write().unwrap() = text.clone();
        }
        if kind == "error" {
            *self.last_error.write().unwrap() = Some(text.clone());
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
    },
    Login,
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
            activate: AtomicBool::new(false),
            target: AtomicIsize::new(0),
            epoch: AtomicU64::new(0),
            config_version: AtomicU64::new(0),
            stop: AtomicBool::new(false),
            sink,
            platform,
            last_status: RwLock::new("Запускаю помощника".into()),
            last_error: RwLock::new(None),
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
                    match execute(&state, &mut music, task) {
                        Ok(message) => {
                            state.emit("info", &message);
                            if state.snapshot().voice_feedback
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
                                if let Err(error) = state.platform.speak(brief) {
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
        *self.shared.settings.write().unwrap() = settings;
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

fn execute(state: &Shared, music: &mut dyn MusicServiceExt, task: Task) -> Result<String> {
    let settings = state.snapshot();
    let (text, target, epoch, preview, dictating) = match task {
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
            mut text,
            target,
            epoch,
            preview,
            samples,
            had_wake,
            dictating,
        } => {
            if state.cancelled(epoch) {
                bail!("Команда отменена");
            }
            if let Some(samples) = samples {
                let refined = crate::speech::whisper(
                    std::path::Path::new(&settings.whisper_executable),
                    std::path::Path::new(&settings.whisper_model),
                    &samples,
                    || state.cancelled(epoch),
                )?;
                text = if had_wake {
                    crate::activation::strip_wake(&refined, &settings.wake_words)
                        .unwrap_or(&refined)
                        .to_string()
                } else {
                    refined
                };
            }
            (text, target, epoch, preview, dictating)
        }
    };
    if state.cancelled(epoch) {
        bail!("Команда отменена");
    }
    state.emit("heard", &text);
    state.emit("status", "Выполняю команду");
    match commands::parse(&text, dictating, &settings.aliases) {
        Command::Search(query) => { browser::open_external(&browser::search_url(&query))?; Ok("Открыт поиск в браузере".into()) }
        Command::Youtube(query) => browser::youtube(&query),
        Command::Music { query, category } => music.play(&settings, &query, &category),
        Command::Generate(prompt) => {
            let output = crate::generator::generate_with_cancel(&settings, &prompt, || state.cancelled(epoch))?;
            insert_if_current(state, &output, target, epoch, preview)?;
            Ok(if preview { "Текст готов в окне результата" } else { "Текст написан" }.into())
        }
        Command::Insert(text) => {
            insert_if_current(state, &text, target, epoch, preview)?;
            Ok(if preview { "Текст готов в окне результата" } else { "Текст вставлен" }.into())
        }
        Command::Pause | Command::Resume | Command::Next => {
            let action = match commands::parse(&text, false, &settings.aliases) {
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
        Command::Unknown(_) => bail!("Команда не распознана. Примеры: «включи Кино», «найди IPv6», «напиши, что такое IPv6»."),
    }
}

pub trait MusicServiceExt: MusicService {
    fn login(&mut self, settings: &Settings) -> Result<String>;
}
impl MusicServiceExt for BrowserMusic {
    fn login(&mut self, settings: &Settings) -> Result<String> {
        BrowserMusic::login(self, settings)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    #[derive(Default)]
    struct Fake {
        inserted: Mutex<Vec<String>>,
    }
    impl Platform for Fake {
        fn foreground(&self) -> isize {
            1
        }
        fn insert(&self, text: &str, _: isize) -> Result<()> {
            self.inserted.lock().unwrap().push(text.into());
            Ok(())
        }
        fn media(&self, _: &str) -> Result<()> {
            Ok(())
        }
        fn volume(&self, _: Option<u8>, _: f32) -> Result<()> {
            Ok(())
        }
        fn speak(&self, _: &str) -> Result<()> {
            Ok(())
        }
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
