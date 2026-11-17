use crate::config::Alias;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Track,
    Artist,
    Playlist,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub enum Command {
    Music { query: String, category: Category },
    OpenMusic(String),
    Youtube(String),
    Search(String),
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
            let start = normalize(alias.phrase.trim_end_matches("{query}").trim_end());
            let pattern = start
                .split_whitespace()
                .map(|s| regex::escape(s).replace('е', "[её]"))
                .collect::<Vec<_>>()
                .join(r"\s+");
            let pattern = regex::Regex::new(&format!(r"(?i)^{}[\s,:]+(.+)$", pattern)).unwrap();
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
            return alias_command(&alias.action, &payload).unwrap_or(Command::Disabled);
        }
    }
    match norm.as_str() {
        "включи яндекс музыку" | "открой яндекс музыку" | "запусти яндекс музыку" => {
            return Command::OpenMusic("yandex".into())
        }
        "включи spotify"
        | "открой spotify"
        | "запусти spotify"
        | "включи спотифай"
        | "открой спотифай" => return Command::OpenMusic("spotify".into()),
        "включи музыку" | "запусти музыку" | "открой музыку" | "включи" =>
        {
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
            "запусти",
            "поставь",
        ],
    ) {
        let mut category = Category::Track;
        let mut query = body;
        for (name, kind) in [
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
        return Command::Music {
            query: query.into(),
            category,
        };
    }
    Command::Unknown(text.into())
}

fn alias_command(action: &str, payload: &str) -> Option<Command> {
    Some(match action {
        "search" => Command::Search(payload.into()),
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
        "volume" => payload
            .trim()
            .parse::<u8>()
            .ok()
            .filter(|v| *v <= 100)
            .map(Command::Volume)
            .unwrap_or_else(|| {
                Command::Invalid("Громкость должна быть целым числом от 0 до 100".into())
            }),
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
