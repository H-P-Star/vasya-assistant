use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Activation {
    Wake,
    Hotkey,
    Both,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpeechMode {
    Vosk,
    Whisper,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LlmProvider {
    LmStudio,
    Ollama,
    OpenRouter,
    OpenAi,
    Compatible,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LlmStyle {
    Neutral,
    Casual,
    Bold,
}

impl LlmProvider {
    pub fn credential_id(&self) -> &'static str {
        match self {
            Self::LmStudio => "lm_studio",
            Self::Ollama => "ollama",
            Self::OpenRouter => "open_router",
            Self::OpenAi => "open_ai",
            Self::Compatible => "compatible",
        }
    }
    pub fn cloud(&self) -> bool {
        !matches!(self, Self::LmStudio | Self::Ollama)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpeechCorrection {
    pub heard: String,
    pub replacement: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BuiltinOverride {
    pub id: String,
    pub enabled: bool,
    pub phrases: Vec<String>,
    pub action: String,
    #[serde(default)]
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VoicePreset {
    pub name: String,
    pub engine: String,
    pub voice: String,
    pub model: String,
    pub speaker: u32,
    pub rate: i32,
    #[serde(default)]
    pub fish_voice: String,
    #[serde(default = "default_fish_model")]
    pub fish_model: String,
}

fn default_fish_model() -> String {
    "s2.1-pro-free".into()
}

fn validate_voice_engine(engine: &str, fish_voice: &str, fish_model: &str) -> Result<()> {
    match engine {
        "windows" | "piper" => Ok(()),
        "fish_audio" => crate::voice::validate_fish_voice(fish_voice, fish_model),
        _ => bail!("Неизвестный движок голоса"),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Alias {
    #[serde(default = "enabled_by_default")]
    pub enabled: bool,
    pub phrase: String,
    pub action: String,
    #[serde(default)]
    pub payload: String,
}

fn enabled_by_default() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub onboarding_complete: bool,
    pub wake_words: Vec<String>,
    pub activation: Activation,
    pub hotkey: String,
    pub microphone: String,
    pub vosk_library: String,
    pub vosk_model: String,
    pub speech_mode: SpeechMode,
    pub whisper_executable: String,
    pub whisper_model: String,
    pub speech_language: String,
    pub speech_hint: String,
    pub speech_gain: f32,
    pub noise_floor: f32,
    pub wake_aliases: Vec<String>,
    pub speech_corrections: Vec<SpeechCorrection>,
    pub llm_repair_speech: bool,
    pub web_search_url: String,
    pub music_service: String,
    pub browser: String,
    pub music_mode: String,
    pub yandex_app_path: String,
    pub spotify_app_path: String,
    pub island_bg: String,
    pub island_edge: String,
    pub island_mic: String,
    pub island_flat: bool,
    pub cue_volume: u16,
    pub cue_path: String,
    pub duck_percent: u16,
    pub recognition_speed: String,
    pub replies: std::collections::BTreeMap<String, String>,
    pub island_enabled: bool,
    pub llm_fallback: bool,
    pub voice_feedback: bool,
    pub tts_engine: String,
    pub tts_voice: String,
    pub tts_volume: u16,
    pub tts_rate: i32,
    pub piper_executable: String,
    pub piper_model: String,
    pub piper_speaker: u32,
    pub fish_voice: String,
    pub fish_model: String,
    pub voice_presets: Vec<VoicePreset>,
    pub start_muted: bool,
    pub start_in_tray: bool,
    pub command_timeout: u64,
    pub llm_provider: LlmProvider,
    pub llm_url: String,
    pub llm_model: String,
    pub llm_structured: bool,
    pub llm_style: LlmStyle,
    pub aliases: Vec<Alias>,
    pub builtin_overrides: Vec<BuiltinOverride>,
}

pub fn data_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("vasya-local"))
        .join("Vasya")
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            onboarding_complete: false,
            wake_words: vec!["вась".into(), "вася".into()],
            activation: Activation::Both,
            hotkey: "Ctrl+Alt+Space".into(),
            microphone: String::new(),
            vosk_library: String::new(),
            vosk_model: String::new(),
            speech_mode: SpeechMode::Whisper,
            whisper_executable: String::new(),
            whisper_model: String::new(),
            speech_language: "auto".into(),
            speech_hint: String::new(),
            speech_gain: 1.0,
            noise_floor: 0.003,
            wake_aliases: Vec::new(),
            speech_corrections: Vec::new(),
            llm_repair_speech: true,
            web_search_url: String::new(),
            music_service: "yandex".into(),
            browser: "edge".into(),
            music_mode: "desktop".into(),
            yandex_app_path: String::new(),
            spotify_app_path: String::new(),
            island_bg: "#192234".into(),
            island_edge: "#b8caff".into(),
            island_mic: "#617aba".into(),
            island_flat: false,
            cue_volume: 20,
            cue_path: String::new(),
            duck_percent: 25,
            recognition_speed: "balanced".into(),
            replies: Default::default(),
            island_enabled: true,
            llm_fallback: true,
            voice_feedback: true,
            tts_engine: "windows".into(),
            tts_voice: String::new(),
            tts_volume: 80,
            tts_rate: 0,
            piper_executable: String::new(),
            piper_model: String::new(),
            piper_speaker: 0,
            fish_voice: String::new(),
            fish_model: default_fish_model(),
            voice_presets: Vec::new(),
            start_muted: false,
            start_in_tray: false,
            command_timeout: 10,
            llm_provider: LlmProvider::LmStudio,
            llm_url: "http://198.18.0.1:1234".into(),
            llm_model: "google/gemma-3-4b".into(),
            llm_structured: true,
            llm_style: LlmStyle::Casual,
            aliases: Vec::new(),
            builtin_overrides: Vec::new(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if !self.web_search_url.trim().is_empty() {
            crate::web_search::searx_endpoint(&self.web_search_url)?;
        }
        if self.wake_words.is_empty() || self.wake_words.iter().any(|w| w.trim().is_empty()) {
            bail!("Укажите хотя бы одно слово активации");
        }
        if !["yandex", "spotify", "youtube"].contains(&self.music_service.as_str()) {
            bail!("Неизвестный музыкальный сервис");
        }
        if !["edge", "chrome"].contains(&self.browser.as_str()) {
            bail!("Неизвестный браузер");
        }
        if self.cue_volume > 100
            || self.duck_percent > 100
            || !matches!(
                self.recognition_speed.as_str(),
                "fast" | "balanced" | "precise"
            )
        {
            bail!("Проверьте громкость сигнала и режим скорости");
        }
        for color in [&self.island_bg, &self.island_edge, &self.island_mic] {
            if color.len() != 7
                || !color.starts_with('#')
                || !color[1..].bytes().all(|b| b.is_ascii_hexdigit())
            {
                bail!("Некорректный цвет острова");
            }
        }
        if !(3..=60).contains(&self.command_timeout) {
            bail!("Ожидание команды: от 3 до 60 секунд");
        }
        if self.hotkey.trim().is_empty() {
            bail!("Укажите горячую клавишу");
        }
        let url = url::Url::parse(&self.llm_url).context("Некорректный адрес сервера генерации")?;
        if !["http", "https"].contains(&url.scheme())
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            bail!("Адрес сервера: http(s)://хост[:порт][/api/v1] без пароля, query и fragment");
        }
        if self.llm_provider.cloud()
            && url.scheme() != "https"
            && ![Some("localhost"), Some("127.0.0.1"), Some("[::1]")].contains(&url.host_str())
        {
            bail!("Для облачного API с ключом нужен HTTPS");
        }
        if self.llm_model.trim().is_empty() {
            bail!("Укажите модель генерации текста");
        }
        if !["desktop", "browser"].contains(&self.music_mode.as_str()) {
            bail!("Неизвестный способ запуска музыки");
        }
        validate_aliases(&self.aliases)?;
        if !["auto", "ru", "en"].contains(&self.speech_language.as_str()) {
            bail!("Язык речи: auto, ru или en");
        }
        if !self.speech_gain.is_finite()
            || !(0.25..=8.0).contains(&self.speech_gain)
            || !self.noise_floor.is_finite()
            || !(0.0..=0.2).contains(&self.noise_floor)
        {
            bail!("Некорректная калибровка микрофона");
        }
        if self.speech_hint.len() > 4000
            || self.wake_aliases.len() > 20
            || self
                .wake_aliases
                .iter()
                .any(|w| w.trim().is_empty() || w.len() > 100)
        {
            bail!("Слишком много вариантов активации или длинная подсказка речи");
        }
        if self.speech_corrections.len() > 100
            || self.speech_corrections.iter().any(|c| {
                c.heard.trim().is_empty()
                    || c.replacement.trim().is_empty()
                    || c.heard.len() > 512
                    || c.replacement.len() > 512
            })
        {
            bail!("Некорректный словарь исправлений речи");
        }
        validate_voice_engine(&self.tts_engine, &self.fish_voice, &self.fish_model)?;
        if self.tts_volume > 100
            || !(-10..=10).contains(&self.tts_rate)
            || self.voice_presets.len() > 32
        {
            bail!("Проверьте параметры голоса");
        }
        for preset in &self.voice_presets {
            validate_voice_engine(&preset.engine, &preset.fish_voice, &preset.fish_model)
                .with_context(|| format!("Проверьте пресет голоса «{}»", preset.name))?;
            if preset.name.trim().is_empty() || !(-10..=10).contains(&preset.rate) {
                bail!("Некорректный пресет голоса");
            }
        }
        let catalog = crate::commands::builtin_catalog();
        let mut ids = std::collections::HashSet::new();
        for rule in &self.builtin_overrides {
            if !catalog.iter().any(|b| b.id == rule.id)
                || !ids.insert(&rule.id)
                || rule.phrases.is_empty()
                || rule.phrases.len() > 32
            {
                bail!("Некорректная встроенная команда");
            }
            validate_aliases(
                &rule
                    .phrases
                    .iter()
                    .map(|phrase| Alias {
                        enabled: rule.enabled,
                        phrase: phrase.clone(),
                        action: rule.action.clone(),
                        payload: rule.payload.clone(),
                    })
                    .collect::<Vec<_>>(),
            )?;
        }
        Ok(())
    }
}

fn validate_aliases(aliases: &[Alias]) -> Result<()> {
    if aliases.len() > 256 {
        bail!("Можно сохранить до 256 команд");
    }
    let mut phrases = std::collections::HashSet::new();
    for alias in aliases {
        let normalized = crate::commands::normalize(&alias.phrase);
        if !phrases.insert(normalized) {
            bail!("Команда «{}» встречается несколько раз", alias.phrase);
        }
        if alias.phrase.len() > 512 || alias.payload.len() > 16000 {
            bail!("Слишком длинная команда");
        }
        if let Some((head, tail)) = alias.phrase.split_once("{query}") {
            if alias.phrase.matches("{query}").count() != 1
                || head.trim().is_empty()
                || !head.chars().next_back().is_some_and(char::is_whitespace)
                || (!tail.is_empty() && !tail.chars().next().is_some_and(char::is_whitespace))
            {
                bail!("Параметр {{query}} нужен один раз, после названия команды; отделяйте его пробелами");
            }
        }
        if alias.phrase.trim().is_empty()
            || ![
                "play_yandex",
                "play_spotify",
                "open_music",
                "open_yandex",
                "open_spotify",
                "cancel",
                "search",
                "web_answer",
                "music",
                "music_artist",
                "music_playlist",
                "youtube",
                "generate",
                "insert",
                "pause",
                "resume",
                "next",
                "volume",
                "mute",
                "volume_up",
                "volume_down",
                "dictation_start",
                "dictation_stop",
                "answer",
                "disabled",
                "launch_app",
            ]
            .contains(&alias.action.as_str())
        {
            bail!("Некорректный пользовательский псевдоним команды");
        }
        if alias.action == "volume"
            && !alias.payload.contains("{query}")
            && alias
                .payload
                .trim()
                .parse::<u8>()
                .ok()
                .is_none_or(|v| v > 100)
        {
            bail!("В содержимом команды громкости укажите целое число от 0 до 100 или {{query}}");
        }
        if alias.action == "launch_app"
            && (alias.payload.contains("{query}") || alias.phrase.contains("{query}"))
        {
            bail!("Для запуска программы укажите точную фразу и постоянный путь к .exe");
        }
        if [
            "search",
            "web_answer",
            "youtube",
            "generate",
            "insert",
            "answer",
            "launch_app",
        ]
        .contains(&alias.action.as_str())
            && alias.payload.trim().is_empty()
            && !alias.phrase.contains("{query}")
        {
            bail!("Заполните содержимое действия команды «{}»", alias.phrase);
        }
    }
    Ok(())
}

pub fn load(path: &Path) -> Result<Settings> {
    if !path.exists() {
        return Ok(Settings::default());
    }
    let settings: Settings = serde_json::from_slice(&std::fs::read(path)?)
        .context("Не удалось прочитать settings.json. Исправьте или переименуйте файл.")?;
    settings.validate()?;
    Ok(settings)
}

pub fn save(settings: &Settings, path: &Path) -> Result<()> {
    settings.validate()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(settings)?)?;
    // Windows rename cannot replace an existing destination; keep a recoverable backup.
    let backup = path.with_extension("bak");
    if path.exists() {
        std::fs::copy(path, &backup)?;
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(&tmp, path).inspect_err(|_| {
        if backup.exists() && !path.exists() {
            let _ = std::fs::copy(&backup, path);
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn multilingual_voice_and_cloud_settings_validate_without_storing_secrets() {
        let mut settings = Settings {
            llm_provider: LlmProvider::OpenRouter,
            llm_url: "https://openrouter.ai/api/v1".into(),
            ..Default::default()
        };
        settings.validate().unwrap();
        assert_eq!(settings.speech_mode, SpeechMode::Whisper);
        let resolved = resolve_speech(&settings, Path::new("installed"));
        assert_eq!(
            Path::new(&resolved.whisper_model),
            Path::new("installed/speech/models/ggml-base.bin")
        );
        assert!(!serde_json::to_value(&settings)
            .unwrap()
            .as_object()
            .unwrap()
            .contains_key("api_key"));
        settings.tts_volume = 101;
        assert!(settings.validate().is_err());
        settings.tts_volume = 70;
        settings.llm_url = "http://untrusted.example/v1".into();
        assert!(settings.validate().is_err());
    }
    #[test]
    fn legacy_settings_keep_aliases_and_new_features_have_defaults() {
        let settings: Settings = serde_json::from_str(
            r#"{"aliases":[{"phrase":"моя музыка","action":"music","payload":"Кино"}]}"#,
        )
        .unwrap();
        settings.validate().unwrap();
        assert!(settings.aliases[0].enabled);
        assert_eq!(settings.music_mode, "desktop");
        assert!(settings.llm_fallback && settings.island_enabled);
    }
    #[test]
    fn duplicate_and_unsafe_command_templates_are_rejected() {
        let mut settings = Settings::default();
        let rule = Alias {
            enabled: true,
            phrase: "Тест".into(),
            action: "pause".into(),
            payload: String::new(),
        };
        settings.aliases = vec![
            rule.clone(),
            Alias {
                phrase: "тест!".into(),
                ..rule.clone()
            },
        ];
        assert!(settings.validate().is_err());
        for phrase in ["{query}", "мой{query}", "мой {query} {query}"] {
            settings.aliases = vec![Alias {
                phrase: phrase.into(),
                ..rule.clone()
            }];
            assert!(settings.validate().is_err(), "{phrase}");
        }
        settings.aliases = vec![Alias {
            phrase: "запусти {query}".into(),
            action: "launch_app".into(),
            ..rule
        }];
        assert!(settings.validate().is_err());
    }
    #[test]
    fn packaged_paths_follow_installation_and_preserve_custom_paths() {
        let mut settings = Settings::default();
        let resolved = resolve_speech(&settings, Path::new("installation"));
        assert_eq!(
            Path::new(&resolved.vosk_library),
            Path::new("installation/speech/vosk/libvosk.dll")
        );
        assert_eq!(
            Path::new(&resolved.vosk_model),
            Path::new("installation/speech/models/vosk-model-small-ru-0.22")
        );
        assert!(settings.vosk_library.is_empty());
        settings.vosk_model = "my-model".into();
        assert_eq!(
            resolve_speech(&settings, Path::new("updated-installation")).vosk_model,
            "my-model"
        );
        settings.vosk_library = data_dir()
            .join("vosk")
            .join("libvosk.dll")
            .to_string_lossy()
            .into();
        settings.vosk_model = data_dir()
            .join("models")
            .join("vosk-model-small-ru-0.22")
            .to_string_lossy()
            .into();
        let migrated = resolve_speech(&settings, Path::new("new"));
        assert_eq!(
            Path::new(&migrated.vosk_library),
            Path::new("new/speech/vosk/libvosk.dll")
        );
        assert_eq!(
            Path::new(&migrated.vosk_model),
            Path::new("new/speech/models/vosk-model-small-ru-0.22")
        );
    }
    #[test]
    fn settings_roundtrip_and_validation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        let mut settings = Settings::default();
        save(&settings, &path).unwrap();
        settings.wake_words = vec!["помощник".into(), "вася".into()];
        save(&settings, &path).unwrap();
        assert_eq!(load(&path).unwrap().wake_words, settings.wake_words);
        settings.llm_url = "file:///secret".into();
        assert!(save(&settings, &path).is_err());
        assert!(load(&path).is_ok());
    }
    #[test]
    fn malformed_config_is_not_silently_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.json");
        std::fs::write(&path, br#"{"voice_feedback":"yes"}"#).unwrap();
        assert!(load(&path).is_err());
    }
}

/// Empty paths select installer resources; explicit custom paths survive upgrades.
pub fn resolve_speech(settings: &Settings, resources: &Path) -> Settings {
    let mut resolved = settings.clone();
    let legacy = data_dir();
    if settings.vosk_library.is_empty()
        || Path::new(&settings.vosk_library) == legacy.join("vosk/libvosk.dll")
    {
        resolved.vosk_library = resources
            .join("speech/vosk/libvosk.dll")
            .to_string_lossy()
            .into();
    }
    if settings.vosk_model.is_empty()
        || Path::new(&settings.vosk_model) == legacy.join("models/vosk-model-small-ru-0.22")
    {
        resolved.vosk_model = resources
            .join("speech/models/vosk-model-small-ru-0.22")
            .to_string_lossy()
            .into();
    }
    if settings.whisper_executable.is_empty() {
        resolved.whisper_executable = resources
            .join(if optimized_whisper_cpu() {
                "speech/whisper/whisper-cli.exe"
            } else {
                "speech/whisper/whisper-cli-baseline.exe"
            })
            .to_string_lossy()
            .into();
    }
    if settings.whisper_model.is_empty() {
        resolved.whisper_model = resources
            .join("speech/models/ggml-base.bin")
            .to_string_lossy()
            .into();
    }
    resolved
}

fn optimized_whisper_cpu() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        std::is_x86_feature_detected!("sse4.2")
            && std::is_x86_feature_detected!("avx2")
            && std::is_x86_feature_detected!("fma")
            && std::is_x86_feature_detected!("f16c")
            && std::is_x86_feature_detected!("bmi2")
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}
