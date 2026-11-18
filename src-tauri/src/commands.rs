use crate::config::Alias;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
pub struct Builtin {
    pub id: &'static str,
    pub title: &'static str,
    pub action: &'static str,
    pub phrases: Vec<&'static str>,
    pub payload: &'static str,
}

pub fn builtin_catalog() -> Vec<Builtin> {
    [
        (
            "music",
            "Музыка",
            "music",
            vec![
                "включи {query}",
                "включит {query}",
                "включить {query}",
                "включите {query}",
                "запусти {query}",
                "поставь {query}",
                "включи музыку {query}",
                "запусти музыку {query}",
                "включи музыку",
                "запусти музыку",
                "включи",
            ],
        ),
        (
            "open_music",
            "Открыть музыкальный сервис",
            "open_music",
            vec!["открой музыку"],
        ),
        (
            "yandex",
            "Открыть Яндекс Музыку",
            "open_yandex",
            vec!["открой яндекс музыку"],
        ),
        (
            "play_yandex",
            "Включить Яндекс Музыку",
            "play_yandex",
            vec!["включи яндекс музыку", "запусти яндекс музыку"],
        ),
        (
            "spotify",
            "Открыть Spotify",
            "open_spotify",
            vec!["открой spotify", "открой спотифай"],
        ),
        (
            "play_spotify",
            "Включить Spotify",
            "play_spotify",
            vec!["включи spotify", "запусти spotify", "включи спотифай"],
        ),
        (
            "pause",
            "Пауза",
            "pause",
            vec!["пауза", "поставь на паузу", "останови музыку"],
        ),
        (
            "resume",
            "Продолжить воспроизведение",
            "resume",
            vec!["продолжи", "продолжай", "продолжи музыку"],
        ),
        (
            "next",
            "Следующий трек",
            "next",
            vec!["следующий", "следующий трек", "переключи трек"],
        ),
        (
            "volume_up",
            "Громче",
            "volume_up",
            vec!["громче", "сделай громче"],
        ),
        (
            "volume_down",
            "Тише",
            "volume_down",
            vec!["тише", "сделай тише"],
        ),
        (
            "volume",
            "Установить громкость",
            "volume",
            vec![
                "громкость {query}",
                "установи громкость {query}",
                "установи громкость на {query}",
                "громкость на {query}",
                "сделай громкость {query}",
                "сделай громкость на {query}",
            ],
        ),
        (
            "search",
            "Поиск в браузере",
            "search",
            vec![
                "найди в браузере {query}",
                "поищи в браузере {query}",
                "найди в интернете {query}",
                "найди {query}",
                "поищи {query}",
            ],
        ),
        (
            "web_answer",
            "Найти ответ в интернете",
            "web_answer",
            vec![
                "узнай в интернете {query}",
                "проверь в интернете {query}",
                "найди ответ в интернете {query}",
                "поищи в интернете {query} и ответь",
                "найди в интернете {query} и ответь",
            ],
        ),
        (
            "youtube",
            "Видео YouTube",
            "youtube",
            vec![
                "включи видео на ютубе {query}",
                "включи на ютубе {query}",
                "найди на ютубе {query}",
                "найди видео на ютубе {query}",
                "покажи на ютубе {query}",
                "включи youtube {query}",
                "включи ютуб {query}",
                "найди youtube {query}",
                "включи видео {query} на ютубе",
                "включи видео {query} на youtube",
                "покажи видео {query} на ютубе",
                "покажи видео {query} на youtube",
                "включи {query} на ютубе",
                "включи {query} на youtube",
                "найди {query} на ютубе",
                "найди {query} на youtube",
            ],
        ),
        (
            "generate",
            "Написать текст с моделью",
            "generate",
            vec!["напиши {query}", "пиши {query}"],
        ),
        (
            "insert",
            "Разовая диктовка",
            "insert",
            vec!["диктуй {query}", "запиши текст {query}"],
        ),
        (
            "dictation_start",
            "Начать диктовку",
            "dictation_start",
            vec!["диктовка", "начни диктовку"],
        ),
        (
            "dictation_stop",
            "Завершить диктовку",
            "dictation_stop",
            vec![
                "стоп диктовка",
                "заверши диктовку",
                "останови диктовку",
                "конец диктовки",
            ],
        ),
        (
            "mute",
            "Отключить микрофон",
            "mute",
            vec!["отключи микрофон", "выключи микрофон"],
        ),
        ("cancel", "Отмена", "cancel", vec!["отмена"]),
    ]
    .into_iter()
    .map(|(id, title, action, phrases)| Builtin {
        id,
        title,
        action,
        phrases,
        payload: if id == "volume" { "{query}" } else { "" },
    })
    .collect()
}

pub fn parse_with_settings(
    text: &str,
    dictating: bool,
    settings: &crate::config::Settings,
) -> Command {
    let text = text.trim();
    if dictating {
        let extra_stops: Vec<_> = settings
            .aliases
            .iter()
            .filter(|r| r.enabled && r.action == "dictation_stop")
            .cloned()
            .chain(
                settings
                    .builtin_overrides
                    .iter()
                    .filter(|r| r.enabled && r.action == "dictation_stop")
                    .flat_map(|r| {
                        r.phrases.iter().map(|p| Alias {
                            enabled: true,
                            phrase: p.clone(),
                            action: "dictation_stop".into(),
                            payload: String::new(),
                        })
                    }),
            )
            .collect();
        if let Some(command) = match_aliases(text, &extra_stops) {
            return command;
        }
        return parse(text, true, &[]);
    }
    if let Some(command) = match_aliases(text, &settings.aliases) {
        return command;
    }
    let catalog = builtin_catalog();
    let mut aliases = Vec::new();
    let mut blockers = Vec::new();
    for builtin in catalog {
        let edited = settings
            .builtin_overrides
            .iter()
            .find(|r| r.id == builtin.id);
        let phrases = edited
            .map(|r| r.phrases.clone())
            .unwrap_or_else(|| builtin.phrases.iter().map(|p| (*p).to_string()).collect());
        let enabled = edited.is_none_or(|r| r.enabled);
        let action = edited.map(|r| r.action.as_str()).unwrap_or(builtin.action);
        let payload = edited
            .map(|r| r.payload.as_str())
            .unwrap_or(builtin.payload);
        for phrase in &phrases {
            aliases.push(Alias {
                enabled: true,
                phrase: phrase.clone(),
                action: if enabled {
                    action.into()
                } else {
                    "disabled".into()
                },
                payload: payload.into(),
            });
        }
        if edited.is_some() {
            for phrase in builtin.phrases {
                if !phrases.iter().any(|p| normalize(p) == normalize(phrase)) {
                    blockers.push(Alias {
                        enabled: true,
                        phrase: phrase.into(),
                        action: "disabled".into(),
                        payload: String::new(),
                    });
                }
            }
        }
    }
    aliases.extend(blockers);
    if let Some((rule, payload)) = match_alias_rule(text, &aliases) {
        if rule.action == "music" {
            return music_body(&payload);
        }
        return alias_command(&rule.action, &payload).unwrap_or(Command::Disabled);
    }
    Command::Unknown(text.into())
}

fn music_body(body: &str) -> Command {
    let mut category = if body.trim().is_empty() {
        Category::Track
    } else {
        Category::Auto
    };
    let mut query = body;
    for (name, kind) in [
        ("группу", Category::Artist),
        ("артиста", Category::Artist),
        ("исполнителя", Category::Artist),
        ("плейлист", Category::Playlist),
        ("трек", Category::Track),
        ("песню", Category::Track),
    ] {
        if let Some(rest) = prefix(body, &[name]) {
            category = kind;
            query = rest;
            break;
        }
    }
    Command::Music {
        query: query.into(),
        category,
    }
}

fn volume_payload(payload: &str) -> Command {
    let norm = normalize(payload);
    let value = norm.trim_end_matches('%').trim();
    let value = [" процентов", " процента", " процент"]
        .into_iter()
        .find_map(|s| value.strip_suffix(s))
        .unwrap_or(value);
    parse_percent(value)
        .map(Command::Volume)
        .unwrap_or_else(|| Command::Invalid("Громкость должна быть от 0 до 100".into()))
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Auto,
    Track,
    Artist,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Command {
    Music { query: String, category: Category },
    OpenMusic(String),
    StartMusic(String),
    Youtube(String),
    Search(String),
    WebAnswer(String),
    Generate(String),
    Insert(String),
    Pause,
    Resume,
    Next,
    Volume(u8),
    VolumeUp,
    VolumeDown,
    DictationStart,
    DictationStop,
    Mute,
    Cancel,
    Answer(String),
    LaunchApp(String),
    Disabled,
    Invalid(String),
    Unknown(String),
}

pub fn time_sensitive(command: &Command) -> bool {
    matches!(command, Command::Music { query, .. } if query.is_empty())
        || matches!(
            command,
            Command::StartMusic(_)
                | Command::Pause
                | Command::Resume
                | Command::Next
                | Command::Volume(_)
                | Command::VolumeUp
                | Command::VolumeDown
                | Command::Mute
                | Command::Cancel
                | Command::DictationStart
                | Command::DictationStop
                | Command::Disabled
                | Command::Invalid(_)
        )
}

pub fn normalize(text: &str) -> String {
    text.to_lowercase()
        .replace('ё', "е")
        .trim_matches(|c: char| c.is_whitespace() || ".,!?;:«»\"".contains(c))
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn prefix<'a>(text: &'a str, phrases: &[&str]) -> Option<&'a str> {
    let lower = text.to_lowercase();
    for phrase in phrases {
        if lower.starts_with(phrase) {
            let end = text
                .char_indices()
                .nth(phrase.chars().count())
                .map(|(i, _)| i)
                .unwrap_or(text.len());
            let rest = &text[end..];
            if rest.starts_with(|c: char| c.is_whitespace() || ",:".contains(c)) {
                let rest = rest.trim_start_matches(|c: char| c.is_whitespace() || ",:".contains(c));
                if !rest.is_empty() {
                    return Some(rest);
                }
            }
        }
    }
    None
}

fn parse_percent(text: &str) -> Option<u8> {
    fn small(word: &str) -> Option<u8> {
        Some(match word {
            "ноль" | "нуль" => 0,
            "один" => 1,
            "два" => 2,
            "три" => 3,
            "четыре" => 4,
            "пять" => 5,
            "шесть" => 6,
            "семь" => 7,
            "восемь" => 8,
            "девять" => 9,
            "десять" => 10,
            "одиннадцать" => 11,
            "двенадцать" => 12,
            "тринадцать" => 13,
            "четырнадцать" => 14,
            "пятнадцать" => 15,
            "шестнадцать" => 16,
            "семнадцать" => 17,
            "восемнадцать" => 18,
            "девятнадцать" => 19,
            "сто" => 100,
            _ => return None,
        })
    }
    fn tens(word: &str) -> Option<u8> {
        Some(match word {
            "двадцать" => 20,
            "тридцать" => 30,
            "сорок" => 40,
            "пятьдесят" => 50,
            "шестьдесят" => 60,
            "семьдесят" => 70,
            "восемьдесят" => 80,
            "девяносто" => 90,
            _ => return None,
        })
    }
    if text.len() <= 3 && text.bytes().all(|c| c.is_ascii_digit()) {
        return text.parse::<u8>().ok().filter(|value| *value <= 100);
    }
    let words: Vec<_> = text.split_whitespace().collect();
    match words.as_slice() {
        [word] => small(word).or_else(|| tens(word)),
        [ten, unit] => {
            let unit = small(unit).filter(|value| (1..=9).contains(value))?;
            Some(tens(ten)? + unit)
        }
        _ => None,
    }
}

pub fn parse(text: &str, dictating: bool, aliases: &[Alias]) -> Command {
    let text = text.trim();
    let norm = normalize(text);
    if [
        "стоп диктовка",
        "заверши диктовку",
        "останови диктовку",
        "конец диктовки",
    ]
    .contains(&norm.as_str())
    {
        return Command::DictationStop;
    }
    if dictating {
        return Command::Insert(format!("{text} "));
    }
    if let Some(command) = match_aliases(text, aliases) {
        return command;
    }
    match norm.as_str() {
        "открой музыку" => return Command::OpenMusic(String::new()),
        "открой яндекс музыку" => return Command::OpenMusic("yandex".into()),
        "включи яндекс музыку" | "запусти яндекс музыку" => {
            return Command::StartMusic("yandex".into())
        }
        "открой spotify" | "открой спотифай" => {
            return Command::OpenMusic("spotify".into())
        }
        "включи spotify" | "запусти spotify" | "включи спотифай" => {
            return Command::StartMusic("spotify".into())
        }
        "включи музыку" | "запусти музыку" | "включи" => {
            return Command::Music {
                query: String::new(),
                category: Category::Track,
            };
        }
        "пауза" | "поставь на паузу" | "останови музыку" => {
            return Command::Pause
        }
        "продолжи" | "продолжай" | "продолжи музыку" => {
            return Command::Resume
        }
        "следующий" | "следующий трек" | "переключи трек" => {
            return Command::Next
        }
        "громче" | "сделай громче" => return Command::VolumeUp,
        "тише" | "сделай тише" => return Command::VolumeDown,
        "диктовка" | "начни диктовку" => return Command::DictationStart,
        "отключи микрофон" | "выключи микрофон" => {
            return Command::Mute
        }
        "отмена" => return Command::Cancel,
        _ => {}
    }
    let volume = regex::Regex::new(
        r"^(?:установи |сделай )?громкость(?: на)? (.+?)(?:\s*(?:%|процент(?:а|ов)?))?$",
    )
    .unwrap();
    if let Some(caps) = volume.captures(&norm) {
        if let Some(value) = parse_percent(caps[1].trim()) {
            return Command::Volume(value);
        }
        return Command::Invalid("Громкость должна быть от 0 до 100".into());
    }
    if let Some(body) = prefix(text, &["напиши", "пиши"]) {
        return Command::Generate(body.into());
    }
    if let Some(body) = prefix(text, &["диктуй", "запиши текст"]) {
        return Command::Insert(body.into());
    }
    if let Some(body) = prefix(
        text,
        &[
            "включи видео на ютубе",
            "включи на ютубе",
            "найди на ютубе",
            "найди видео на ютубе",
            "покажи на ютубе",
            "включи ютуб",
            "включи youtube",
            "найди youtube",
        ],
    ) {
        return Command::Youtube(body.into());
    }
    for suffix in [" на ютубе", " на youtube"] {
        if normalize(text).ends_with(suffix) {
            if let Some(body) = prefix(text, &["включи видео", "покажи видео", "включи", "найди"])
            {
                let count = body.chars().count().saturating_sub(suffix.chars().count());
                let query: String = body.chars().take(count).collect();
                if !query.trim().is_empty() {
                    return Command::Youtube(query.trim().into());
                }
            }
        }
    }
    if let Some(body) = prefix(
        text,
        &[
            "узнай в интернете",
            "проверь в интернете",
            "найди ответ в интернете",
        ],
    ) {
        return Command::WebAnswer(body.into());
    }
    if let Some(body) = prefix(
        text,
        &[
            "найди в браузере",
            "поищи в браузере",
            "найди в интернете",
            "найди",
            "поищи",
        ],
    ) {
        return Command::Search(body.into());
    }
    if let Some(body) = prefix(
        text,
        &[
            "включи музыку",
            "запусти музыку",
            "включи",
            "включит",
            "включить",
            "включите",
            "запусти",
            "поставь",
        ],
    ) {
        return music_body(body);
    }
    Command::Unknown(text.into())
}

fn match_aliases(text: &str, aliases: &[Alias]) -> Option<Command> {
    let (alias, payload) = match_alias_rule(text, aliases)?;
    Some(alias_command(&alias.action, &payload).unwrap_or(Command::Disabled))
}

fn match_alias_rule<'a>(text: &str, aliases: &'a [Alias]) -> Option<(&'a Alias, String)> {
    let norm = normalize(text);
    // Exact overrides win over templates, irrespective of editor row order.
    let mut ordered: Vec<_> = aliases.iter().filter(|a| a.enabled).collect();
    ordered.sort_by_key(|a| {
        (
            a.phrase.contains("{query}"),
            std::cmp::Reverse(a.phrase.len()),
        )
    });
    for alias in ordered {
        let template = alias.phrase.contains("{query}");
        let captured = if template {
            let (head, tail) = alias.phrase.split_once("{query}").unwrap();
            // Most templates can be ruled out without constructing a Unicode regex.
            if let Some(first) = head.split_whitespace().next() {
                if !norm.starts_with(&normalize(first)) {
                    continue;
                }
            }
            thread_local! {
                static CACHE: std::cell::RefCell<std::collections::HashMap<String, regex::Regex>> = Default::default();
            }
            let pattern = CACHE.with_borrow_mut(|cache| {
                if cache.len() >= 256 {
                    cache.clear();
                }
                cache
                    .entry(alias.phrase.clone())
                    .or_insert_with(|| {
                        let pattern_for = |s: &str| {
                            normalize(s)
                                .split_whitespace()
                                .map(|s| regex::escape(s).replace('е', "[её]"))
                                .collect::<Vec<_>>()
                                .join(r"\s+")
                        };
                        let head = pattern_for(head);
                        let tail = pattern_for(tail);
                        let suffix = if tail.is_empty() {
                            "(.+)$".to_string()
                        } else {
                            format!(r"(.+?)\s+{}[.!?]*$", tail)
                        };
                        regex::Regex::new(&format!(r"(?i)^{}[\s,:]+{}", head, suffix)).unwrap()
                    })
                    .clone()
            });
            pattern
                .captures(text)
                .and_then(|caps| caps.get(1).map(|m| m.as_str()))
        } else if normalize(&alias.phrase) == norm {
            Some("")
        } else {
            None
        };
        if let Some(query) = captured {
            let payload = if alias.payload.is_empty() {
                query.to_string()
            } else {
                alias.payload.replace("{query}", query)
            };
            return Some((alias, payload));
        }
    }
    None
}

fn alias_command(action: &str, payload: &str) -> Option<Command> {
    Some(match action {
        "search" => Command::Search(payload.into()),
        "web_answer" => Command::WebAnswer(payload.into()),
        "youtube" => Command::Youtube(payload.into()),
        "music" | "music_artist" | "music_playlist" => Command::Music {
            query: payload.into(),
            category: match action {
                "music_artist" => Category::Artist,
                "music_playlist" => Category::Playlist,
                _ => Category::Track,
            },
        },
        "generate" => Command::Generate(payload.into()),
        "insert" => Command::Insert(payload.into()),
        "pause" => Command::Pause,
        "resume" => Command::Resume,
        "next" => Command::Next,
        "mute" => Command::Mute,
        "volume_up" => Command::VolumeUp,
        "volume_down" => Command::VolumeDown,
        "play_yandex" => Command::StartMusic("yandex".into()),
        "play_spotify" => Command::StartMusic("spotify".into()),
        "open_music" => Command::OpenMusic(String::new()),
        "open_yandex" => Command::OpenMusic("yandex".into()),
        "open_spotify" => Command::OpenMusic("spotify".into()),
        "cancel" => Command::Cancel,
        "volume" => volume_payload(payload),
        "dictation_start" => Command::DictationStart,
        "dictation_stop" => Command::DictationStop,
        "answer" => Command::Answer(payload.into()),
        "disabled" => Command::Disabled,
        "launch_app" => Command::LaunchApp(payload.into()),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn web_answer_uses_its_own_command_and_preserves_browser_search() {
        let settings = crate::config::Settings::default();
        for (phrase, expected) in [
            (
                "узнай в интернете, что такое IPv6",
                Command::WebAnswer("что такое IPv6".into()),
            ),
            (
                "поищи в интернете IPv6 и ответь",
                Command::WebAnswer("IPv6".into()),
            ),
            ("найди в интернете IPv6", Command::Search("IPv6".into())),
            ("найди IPv6 на ютубе", Command::Youtube("IPv6".into())),
        ] {
            assert_eq!(
                parse_with_settings(phrase, false, &settings),
                expected,
                "{phrase}"
            );
        }
    }
    #[test]
    fn changing_music_does_not_swallow_youtube_and_old_video_phrases_stay_blocked() {
        use crate::config::{BuiltinOverride, Settings};
        let mut settings = Settings {
            builtin_overrides: vec![BuiltinOverride {
                id: "music".into(),
                enabled: false,
                phrases: vec!["включи {query}".into()],
                action: "music".into(),
                payload: String::new(),
            }],
            ..Default::default()
        };
        assert_eq!(
            parse_with_settings("включи видео котов на ютубе", false, &settings),
            Command::Youtube("котов".into())
        );
        assert_eq!(
            parse_with_settings("включи музыку Кино", false, &settings),
            Command::Disabled
        );
        settings.builtin_overrides.push(BuiltinOverride {
            id: "youtube".into(),
            enabled: true,
            phrases: vec!["смотри {query}".into()],
            action: "youtube".into(),
            payload: String::new(),
        });
        assert_eq!(
            parse_with_settings("включи видео котов на ютубе", false, &settings),
            Command::Disabled
        );
        assert_eq!(
            parse_with_settings("смотри котов", false, &settings),
            Command::Youtube("котов".into())
        );
    }
    #[test]
    fn catalog_routing_preserves_standard_commands() {
        let settings = crate::config::Settings::default();
        for text in [
            "включи музыку Кино",
            "включи исполнителя Кино",
            "включи трек Перемен исполнителя Кино",
            "включи плейлист Работа",
            "найди IPv6",
            "включи видео котов на ютубе",
            "покажи видео котов на youtube",
            "сделай громкость на сорок два процента",
            "громкость 35%",
            "пиши про IPv6",
            "диктуй Привет!",
            "пауза",
            "следующий трек",
            "отключи микрофон",
            "отмена",
        ] {
            assert_eq!(
                parse_with_settings(text, false, &settings),
                parse(text, false, &[]),
                "{text}"
            );
        }
    }
    #[test]
    fn edited_builtins_replace_old_phrases_and_restore_without_llm() {
        use crate::config::{BuiltinOverride, Settings};
        let mut settings = Settings::default();
        settings.builtin_overrides.push(BuiltinOverride {
            id: "pause".into(),
            enabled: true,
            phrases: vec!["замри музыка".into()],
            action: "pause".into(),
            payload: String::new(),
        });
        assert_eq!(
            parse_with_settings("замри музыка", false, &settings),
            Command::Pause
        );
        assert_eq!(
            parse_with_settings("пауза", false, &settings),
            Command::Disabled
        );
        settings.builtin_overrides[0].enabled = false;
        assert_eq!(
            parse_with_settings("замри музыка", false, &settings),
            Command::Disabled
        );
        settings.builtin_overrides.clear();
        assert_eq!(
            parse_with_settings("пауза", false, &settings),
            Command::Pause
        );
    }
    #[test]
    fn edited_builtin_templates_keep_specificity_and_spoken_numbers() {
        use crate::config::{BuiltinOverride, Settings};
        let settings = Settings {
            builtin_overrides: vec![
                BuiltinOverride {
                    id: "music".into(),
                    enabled: true,
                    phrases: vec!["включи {query}".into()],
                    action: "music".into(),
                    payload: String::new(),
                },
                BuiltinOverride {
                    id: "youtube".into(),
                    enabled: true,
                    phrases: vec!["включи видео {query}".into()],
                    action: "youtube".into(),
                    payload: String::new(),
                },
                BuiltinOverride {
                    id: "volume".into(),
                    enabled: true,
                    phrases: vec!["уровень {query}".into()],
                    action: "volume".into(),
                    payload: "{query}".into(),
                },
            ],
            ..Settings::default()
        };
        settings.validate().unwrap();
        assert_eq!(
            parse_with_settings("включи видео котов", false, &settings),
            Command::Youtube("котов".into())
        );
        assert_eq!(
            parse_with_settings("уровень тридцать пять процентов", false, &settings),
            Command::Volume(35)
        );
        assert!(matches!(
            parse_with_settings("включи исполнителя Кино", false, &settings),
            Command::Music {
                category: Category::Artist,
                ..
            }
        ));
        assert_eq!(
            parse_with_settings("стоп диктовка", true, &settings),
            Command::DictationStop
        );
    }
    #[test]
    fn opening_a_player_and_starting_music_are_distinct() {
        let settings = crate::config::Settings::default();
        for (phrase, command) in [
            ("открой музыку", Command::OpenMusic(String::new())),
            (
                "включи музыку",
                Command::Music {
                    query: String::new(),
                    category: Category::Track,
                },
            ),
            ("включи яндекс музыку", Command::StartMusic("yandex".into())),
            ("включи спотифай", Command::StartMusic("spotify".into())),
        ] {
            assert_eq!(parse_with_settings(phrase, false, &settings), command);
            assert_eq!(parse(phrase, false, &[]), command);
        }
    }
    #[test]
    fn custom_rules_override_templates_and_can_be_disabled() {
        let mut rules = vec![
            Alias {
                enabled: true,
                phrase: "Включи {query}".into(),
                action: "search".into(),
                payload: "{query} официальный сайт".into(),
            },
            Alias {
                enabled: true,
                phrase: "включи музыку".into(),
                action: "disabled".into(),
                payload: String::new(),
            },
        ];
        assert_eq!(parse("Включи музыку!", false, &rules), Command::Disabled);
        assert_eq!(
            parse("Включи  IPv6 & Кино", false, &rules),
            Command::Search("IPv6 & Кино официальный сайт".into())
        );
        assert!(matches!(
            parse("включиfoo", false, &rules),
            Command::Unknown(_)
        ));
        rules[0].enabled = false;
        assert!(matches!(
            parse("включи Кино", false, &rules),
            Command::Music { .. }
        ));
        rules.clear();
        assert_eq!(
            parse("открой Яндекс Музыку", false, &rules),
            Command::OpenMusic("yandex".into())
        );
        assert_eq!(
            parse("открой Spotify", false, &rules),
            Command::OpenMusic("spotify".into())
        );
        assert_eq!(
            parse("включи музыку", false, &rules),
            Command::Music {
                query: String::new(),
                category: Category::Track
            }
        );
    }
    #[test]
    fn longer_template_wins_and_dictation_remains_literal() {
        let rules = vec![
            Alias {
                enabled: true,
                phrase: "мой {query}".into(),
                action: "search".into(),
                payload: String::new(),
            },
            Alias {
                enabled: true,
                phrase: "мой плейлист {query}".into(),
                action: "music_playlist".into(),
                payload: String::new(),
            },
        ];
        assert_eq!(
            parse("мой плейлист Для работы", false, &rules),
            Command::Music {
                query: "Для работы".into(),
                category: Category::Playlist
            }
        );
        assert_eq!(
            parse("мой плейлист Работа", true, &rules),
            Command::Insert("мой плейлист Работа ".into())
        );
    }
    #[test]
    fn generation_is_distinct_from_dictation_and_preserves_case() {
        assert_eq!(
            parse("напиши, что такое IPv6", false, &[]),
            Command::Generate("что такое IPv6".into())
        );
        assert_eq!(
            parse("диктуй Привет, Анна!", false, &[]),
            Command::Insert("Привет, Анна!".into())
        );
        assert_eq!(
            parse("напиши что такое IPv6", true, &[]),
            Command::Insert("напиши что такое IPv6 ".into())
        );
        assert_eq!(parse("стоп диктовка", true, &[]), Command::DictationStop);
    }
    #[test]
    fn video_routing_precedes_generic_search_and_music() {
        for phrase in [
            "найди на ютубе котов",
            "включи видео котов на ютубе",
            "включи youtube котов",
        ] {
            assert_eq!(parse(phrase, false, &[]), Command::Youtube("котов".into()));
        }
        assert_eq!(
            parse("найди в браузере IPv6", false, &[]),
            Command::Search("IPv6".into())
        );
    }
    #[test]
    fn music_categories_volume_and_aliases() {
        assert_eq!(
            parse("включи плейлист Работа", false, &[]),
            Command::Music {
                query: "Работа".into(),
                category: Category::Playlist
            }
        );
        assert_eq!(
            parse("включи исполнителя Кино", false, &[]),
            Command::Music {
                query: "Кино".into(),
                category: Category::Artist
            }
        );
        assert_eq!(
            parse("громкость 35 процентов", false, &[]),
            Command::Volume(35)
        );
        assert!(matches!(
            parse("громкость 999", false, &[]),
            Command::Invalid(_)
        ));
        let alias = Alias {
            enabled: true,
            phrase: "мой трек".into(),
            action: "music".into(),
            payload: "Перемен".into(),
        };
        assert!(matches!(
            parse("мой трек", false, &[alias]),
            Command::Music { .. }
        ));
        assert!(matches!(parse("пиши", false, &[]), Command::Unknown(_)));
    }

    #[test]
    fn absolute_volume_accepts_russian_asr_numbers() {
        for (phrase, value) in [
            ("громкость ноль", 0),
            ("сделай громкость на нуль процентов", 0),
            ("громкость один процент", 1),
            ("громкость три процента", 3),
            ("громкость одиннадцать процентов", 11),
            ("установи громкость двадцать", 20),
            ("громкость тридцать пять процентов", 35),
            ("сделай громкость на сорок два процента", 42),
            ("громкость девяносто девять", 99),
            ("громкость сто процентов", 100),
            ("громкость 35%", 35),
            ("громкость 100 процентов", 100),
        ] {
            assert_eq!(
                parse(phrase, false, &[]),
                Command::Volume(value),
                "{phrase}"
            );
        }
        for phrase in [
            "громкость сто один",
            "громкость двести",
            "громкость минус пять",
            "громкость тридцать ноль",
            "громкость десять пять",
            "громкость 101",
        ] {
            assert!(
                matches!(parse(phrase, false, &[]), Command::Invalid(_)),
                "{phrase}"
            );
        }
    }
}
