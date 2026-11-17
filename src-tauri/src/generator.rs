use crate::config::{LlmProvider, Settings};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

fn client(seconds: u64) -> Result<reqwest::blocking::Client> {
    Ok(reqwest::blocking::Client::builder()
        .no_proxy()
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(seconds))
        .build()?)
}

pub fn generate(settings: &Settings, prompt: &str) -> Result<String> {
    generate_with_cancel(settings, prompt, || false)
}

pub fn generate_with_cancel(
    settings: &Settings,
    prompt: &str,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    request(
        settings,
        prompt,
        "Отвечай по-русски, если не просят иначе. Напиши только запрошенный текст.",
        false,
        cancelled,
    )
}

fn request(
    settings: &Settings,
    prompt: &str,
    system: &str,
    json_mode: bool,
    cancelled: impl Fn() -> bool,
) -> Result<String> {
    let (path, mut body) = match settings.llm_provider {
        LlmProvider::LmStudio => (
            "/v1/chat/completions",
            json!({
                "model": settings.llm_model, "stream": false, "max_tokens": 1024,
                "messages": [{"role":"system","content":system},{"role":"user","content":prompt}],
            }),
        ),
        LlmProvider::Ollama => (
            "/api/generate",
            json!({
                "model": settings.llm_model, "prompt": prompt, "stream": false,
                "system": system, "options": {"num_predict":1024},
            }),
        ),
    };
    if json_mode {
        match settings.llm_provider {
            LlmProvider::LmStudio => {
                body["response_format"] = json!({"type":"json_schema", "json_schema": {
                    "name":"vasya_intent", "strict":true, "schema": {
                        "type":"object", "additionalProperties":false, "required":["action"],
                        "properties": {
                            "action":{"type":"string","enum":["music","search","youtube","pause","resume","next","volume_up","volume_down","volume","dictation_start","dictation_stop","mute","insert","answer"]},
                            "query":{"type":"string"}, "text":{"type":"string"},
                            "category":{"type":"string","enum":["track","artist","playlist"]},
                            "value":{"type":"integer","minimum":0,"maximum":100}
                        }
                    }
                }})
            }
            LlmProvider::Ollama => body["format"] = json!("json"),
        }
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let response: Value = runtime.block_on(async {
        let client = reqwest::Client::builder()
            .no_proxy()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(90))
            .build()?;
        let mut request = Box::pin(async {
            Ok::<Value, anyhow::Error>(
                client
                    .post(format!("{}{path}", settings.llm_url.trim_end_matches('/')))
                    .json(&body)
                    .send()
                    .await
                    .context(
                        "Сервер генерации недоступен. Проверьте запуск LM Studio/Ollama и адрес",
                    )?
                    .error_for_status()
                    .context(
                        "Сервер отклонил запрос. Проверьте провайдер и имя загруженной модели",
                    )?
                    .json()
                    .await?,
            )
        });
        loop {
            if cancelled() {
                bail!("Генерация отменена");
            }
            if let Ok(result) = tokio::time::timeout(Duration::from_millis(50), &mut request).await
            {
                return result;
            }
        }
    })?;
    let text = match settings.llm_provider {
        LlmProvider::LmStudio => response["choices"][0]["message"]["content"].as_str(),
        LlmProvider::Ollama => response["response"].as_str(),
    }
    .unwrap_or_default()
    .trim();
    if text.is_empty() {
        bail!("Модель вернула пустой текст");
    }
    Ok(text.into())
}

pub fn resolve(
    settings: &Settings,
    utterance: &str,
    cancelled: impl Fn() -> bool,
) -> Result<crate::commands::Command> {
    let system = r#"Ты голосовой помощник Вася. Определи намерение пользователя и верни ТОЛЬКО JSON.
Разрешены следующие формы:
{"action":"music","query":"название","category":"track"} category: track/artist/playlist.
{"action":"search","query":"запрос"}, {"action":"youtube","query":"запрос"}.
{"action":"pause"}, {"action":"resume"}, {"action":"next"},
{"action":"volume_up"}, {"action":"volume_down"}, {"action":"volume","value":35},
{"action":"dictation_start"}, {"action":"dictation_stop"}, {"action":"mute"}.
Если пользователь явно просит написать/вставить текст: {"action":"insert","text":"готовый текст"}.
Если вопрос, разговор или подходящего действия нет: {"action":"answer","text":"содержательный ответ по-русски"}.
Не выдумывай действий, путей программ, shell-команд. Не запускай музыку при просьбе просто объяснить что-то.
При неясном запросе ответь уточняющим вопросом через answer. value только целое от 0 до 100."#;
    let text = request(settings, utterance, system, true, cancelled)?;
    parse_resolution(&text)
}

fn parse_resolution(text: &str) -> Result<crate::commands::Command> {
    use crate::commands::{Category, Command};
    let text = text.trim();
    let text = text
        .strip_prefix("```json")
        .or_else(|| text.strip_prefix("```"))
        .and_then(|text| text.strip_suffix("```"))
        .unwrap_or(text)
        .trim();
    let plan: Value = serde_json::from_str(text)
        .context("LLM вернула некорректный план. Действие не выполнено")?;
    let field = |name: &str| -> Result<String> {
        let value = plan[name]
            .as_str()
            .context("LLM не указала текст действия")?
            .trim();
        if value.is_empty() || value.chars().count() > 16000 {
            bail!("LLM вернула пустой или слишком длинный текст");
        }
        Ok(value.into())
    };
    Ok(match plan["action"].as_str() {
        Some("music") => Command::Music {
            query: field("query")?,
            category: match plan["category"].as_str().unwrap_or("track") {
                "track" => Category::Track,
                "artist" => Category::Artist,
                "playlist" => Category::Playlist,
                _ => bail!("LLM вернула неизвестную категорию музыки"),
            },
        },
        Some("search") => Command::Search(field("query")?),
        Some("youtube") => Command::Youtube(field("query")?),
        Some("pause") => Command::Pause,
        Some("resume") => Command::Resume,
        Some("next") => Command::Next,
        Some("volume_up") => Command::VolumeUp,
        Some("volume_down") => Command::VolumeDown,
        Some("volume") => Command::Volume(
            plan["value"]
                .as_u64()
                .filter(|v| *v <= 100)
                .context("LLM указала недопустимую громкость")? as u8,
        ),
        Some("dictation_start") => Command::DictationStart,
        Some("dictation_stop") => Command::DictationStop,
        Some("mute") => Command::Mute,
        Some("insert") => Command::Insert(field("text")?),
        Some("answer") => Command::Answer(field("text")?),
        _ => bail!("LLM предложила неподдерживаемое действие. Оно не выполнено"),
    })
}

pub fn models(settings: &Settings) -> Result<Vec<String>> {
    let path = match settings.llm_provider {
        LlmProvider::LmStudio => "/v1/models",
        LlmProvider::Ollama => "/api/tags",
    };
    let response: Value = client(10)?
        .get(format!("{}{path}", settings.llm_url.trim_end_matches('/')))
        .send()
        .context("Не удалось подключиться к серверу генерации")?
        .error_for_status()?
        .json()?;
    let (key, field) = match settings.llm_provider {
        LlmProvider::LmStudio => ("data", "id"),
        LlmProvider::Ollama => ("models", "name"),
    };
    Ok(response[key]
        .as_array()
        .context("Неожиданный формат списка моделей")?
        .iter()
        .filter_map(|v| v[field].as_str().map(String::from))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn intent_plans_select_only_supported_actions_or_answers() {
        use crate::commands::{Category, Command};
        assert_eq!(
            parse_resolution(r#"{"action":"music","query":"Кино","category":"artist"}"#).unwrap(),
            Command::Music {
                query: "Кино".into(),
                category: Category::Artist
            }
        );
        assert_eq!(
            parse_resolution(r#"{"action":"pause"}"#).unwrap(),
            Command::Pause
        );
        assert_eq!(
            parse_resolution("```json\n{\"action\":\"answer\",\"text\":\"IPv6 — протокол\"}\n```")
                .unwrap(),
            Command::Answer("IPv6 — протокол".into())
        );
        for plan in [
            r#"{"action":"shell","command":"calc.exe"}"#,
            r#"{"action":"launch_app","text":"calc.exe"}"#,
            r#"{"action":"volume","value":101}"#,
            r#"{"action":"volume","value":-1}"#,
            r#"{"action":"volume","value":0.5}"#,
            r#"{"action":"answer","text":" "}"#,
            r#"{"action":"music","query":"Кино","category":"unknown"}"#,
            "not JSON",
        ] {
            assert!(parse_resolution(plan).is_err(), "{plan}");
        }
    }
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };

    fn exercise(
        provider: LlmProvider,
        response: &str,
        path: &str,
        inspect: fn(Value),
    ) -> Result<String> {
        exercise_request(provider, response, path, inspect, false)
    }

    fn exercise_request(
        provider: LlmProvider,
        response: &str,
        path: &str,
        inspect: fn(Value),
        json_mode: bool,
    ) -> Result<String> {
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let response = response.to_owned();
        let path = path.to_owned();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert_ne!(n, 0);
                bytes.extend_from_slice(&buffer[..n]);
                let request = String::from_utf8_lossy(&bytes);
                if let Some((headers, body)) = request.split_once("\r\n\r\n") {
                    let len = headers
                        .lines()
                        .find_map(|line| {
                            line.to_lowercase()
                                .strip_prefix("content-length:")
                                .and_then(|v| v.trim().parse::<usize>().ok())
                        })
                        .unwrap();
                    if body.len() >= len {
                        break;
                    }
                }
            }
            let request = String::from_utf8(bytes).unwrap();
            assert!(request.starts_with(&format!("POST {path}")));
            let body: Value =
                serde_json::from_str(request.split_once("\r\n\r\n").unwrap().1).unwrap();
            assert_eq!(body["stream"], false);
            assert!(body.get("tools").is_none());
            inspect(body);
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
        });
        let settings = Settings {
            llm_url: format!("http://{address}"),
            llm_provider: provider,
            ..Settings::default()
        };
        let result = request(&settings, "что такое IPv6", "system", json_mode, || false);
        thread.join().unwrap();
        result
    }
    #[test]
    fn intent_requests_use_supported_structured_formats() {
        let text = exercise_request(
            LlmProvider::LmStudio,
            r#"{"choices":[{"message":{"content":"{\"action\":\"pause\"}"}}]}"#,
            "/v1/chat/completions",
            |body| {
                assert_eq!(body["response_format"]["type"], "json_schema");
                let schema = &body["response_format"]["json_schema"]["schema"];
                assert_eq!(schema["additionalProperties"], false);
                assert!(!schema["properties"]["action"]["enum"]
                    .as_array()
                    .unwrap()
                    .contains(&json!("launch_app")));
            },
            true,
        )
        .unwrap();
        assert_eq!(
            parse_resolution(&text).unwrap(),
            crate::commands::Command::Pause
        );
        exercise_request(
            LlmProvider::Ollama,
            r#"{"response":"{\"action\":\"pause\"}"}"#,
            "/api/generate",
            |body| assert_eq!(body["format"], "json"),
            true,
        )
        .unwrap();
    }
    #[test]
    fn lm_studio_uses_chat_completions() {
        let text = exercise(
            LlmProvider::LmStudio,
            r#"{"choices":[{"message":{"content":"IPv6 — сетевой протокол."}}]}"#,
            "/v1/chat/completions",
            |body| {
                assert_eq!(body["messages"][1]["content"], "что такое IPv6");
                assert_eq!(body["model"], "google/gemma-3-4b");
                assert_eq!(body["max_tokens"], 1024);
            },
        )
        .unwrap();
        assert!(text.contains("IPv6"));
    }
    #[test]
    fn ollama_uses_native_generate() {
        let text = exercise(
            LlmProvider::Ollama,
            r#"{"response":"IPv6 — сетевой протокол."}"#,
            "/api/generate",
            |body| {
                assert_eq!(body["prompt"], "что такое IPv6");
            },
        )
        .unwrap();
        assert!(text.contains("IPv6"));
    }
    #[test]
    fn empty_model_response_is_an_error() {
        assert!(exercise(
            LlmProvider::LmStudio,
            r#"{"choices":[]}"#,
            "/v1/chat/completions",
            |_| {}
        )
        .is_err());
    }

    #[test]
    fn cancellation_does_not_wait_for_the_model_timeout() {
        use std::time::Instant;
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let thread = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut buffer = [0; 4096];
            // Do not answer: the client must cancel instead of waiting 90 seconds.
            while let Ok(n) = stream.read(&mut buffer) {
                if n == 0 {
                    break;
                }
            }
        });
        let settings = Settings {
            llm_url: format!("http://{address}"),
            ..Settings::default()
        };
        let start = Instant::now();
        let result = generate_with_cancel(&settings, "test", || {
            start.elapsed() > Duration::from_millis(100)
        });
        assert!(result.unwrap_err().to_string().contains("отменена"));
        assert!(start.elapsed() < Duration::from_secs(2));
        thread.join().unwrap();
    }
}
