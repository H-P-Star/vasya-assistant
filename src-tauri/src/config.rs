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
    pub music_service: String,
    pub browser: String,
    pub music_mode: String,
    pub yandex_app_path: String,
    pub spotify_app_path: String,
    pub island_enabled: bool,
    pub llm_fallback: bool,
    pub voice_feedback: bool,
    pub start_muted: bool,
    pub start_in_tray: bool,
    pub command_timeout: u64,
    pub llm_provider: LlmProvider,
    pub llm_url: String,
    pub llm_model: String,
    pub aliases: Vec<Alias>,
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
            speech_mode: SpeechMode::Vosk,
            whisper_executable: String::new(),
            whisper_model: String::new(),
            music_service: "yandex".into(),
            browser: "edge".into(),
            music_mode: "desktop".into(),
            yandex_app_path: String::new(),
            spotify_app_path: String::new(),
            island_enabled: true,
            llm_fallback: true,
            voice_feedback: true,
            start_muted: false,
            start_in_tray: false,
            command_timeout: 10,
            llm_provider: LlmProvider::LmStudio,
            llm_url: "http://198.18.0.1:1234".into(),
            llm_model: "google/gemma-3-4b".into(),
            aliases: Vec::new(),
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<()> {
        if self.wake_words.is_empty() || self.wake_words.iter().any(|w| w.trim().is_empty()) {
            bail!("Укажите хотя бы одно слово активации");
        }
        if !["yandex", "spotify", "youtube"].contains(&self.music_service.as_str()) {
            bail!("Неизвестный музыкальный сервис");
        }
        if !["edge", "chrome"].contains(&self.browser.as_str()) {
            bail!("Неизвестный браузер");
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
            || url.path() != "/"
        {
            bail!("Адрес сервера: http(s)://хост:порт без пути и пароля; /v1 добавляется автоматически");
        }
        if self.llm_model.trim().is_empty() {
            bail!("Укажите модель генерации текста");
        }
        if !["desktop", "browser"].contains(&self.music_mode.as_str()) {
            bail!("Неизвестный способ запуска музыки");
        }
        if self.aliases.len() > 256 {
            bail!("Можно сохранить до 256 команд");
        }
        let mut phrases = std::collections::HashSet::new();
        for alias in &self.aliases {
            let normalized = crate::commands::normalize(&alias.phrase);
            if !phrases.insert(normalized) {
                bail!("Команда «{}» встречается несколько раз", alias.phrase);
            }
            if alias.phrase.len() > 512 || alias.payload.len() > 16000 {
                bail!("Слишком длинная команда");
            }
            if alias.phrase.contains("{query}")
                && (!alias.phrase.ends_with("{query}")
                    || alias.phrase.matches("{query}").count() != 1
                    || alias
                        .phrase
                        .trim_end_matches("{query}")
                        .chars()
                        .last()
                        .is_none_or(|c| !c.is_whitespace()))
            {
                bail!(
                    "Параметр {{query}} должен находиться один раз в конце фразы после её названия"
                );
            }
            if alias.phrase.trim().is_empty()
                || ![
                    "search",
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
                bail!(
                    "В содержимом команды громкости укажите целое число от 0 до 100 или {{query}}"
                );
            }
            if alias.action == "launch_app"
                && (alias.payload.contains("{query}") || alias.phrase.contains("{query}"))
            {
                bail!("Для запуска программы укажите точную фразу и постоянный путь к .exe");
            }
            if [
                "search",
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
        for phrase in ["{query}", "мой {query} запрос", "мой {query} {query}"] {
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
    resolved
}
