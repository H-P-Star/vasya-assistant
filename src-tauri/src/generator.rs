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
    let system = "Отвечай по-русски, если не просят иначе. Напиши только запрошенный текст.";
    let (path, body) = match settings.llm_provider {
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
        let result = generate(&settings, "что такое IPv6");
        thread.join().unwrap();
        result
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
