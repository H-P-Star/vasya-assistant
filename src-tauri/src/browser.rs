use crate::{commands::Category, config::Settings};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    net::TcpStream,
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

pub fn search_url(query: &str) -> String {
    let mut url = url::Url::parse("https://www.google.com/search").unwrap();
    url.query_pairs_mut().append_pair("q", query);
    url.into()
}

pub fn youtube_search_url(query: &str) -> String {
    let mut url = url::Url::parse("https://www.youtube.com/results").unwrap();
    url.query_pairs_mut().append_pair("search_query", query);
    url.into()
}

pub fn open_external(url: &str) -> Result<()> {
    let parsed = url::Url::parse(url)?;
    if !["http", "https"].contains(&parsed.scheme()) {
        bail!("Разрешены только HTTP(S) ссылки");
    }
    #[cfg(windows)]
    return crate::platform::open_url(url);
    #[cfg(not(windows))]
    bail!("Открытие браузера поддерживается только в Windows");
}

fn find_video(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(id) = map.get("videoRenderer").and_then(|v| v["videoId"].as_str()) {
                if id.len() == 11
                    && id
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                {
                    return Some(id.into());
                }
            }
            map.iter()
                .filter(|(k, _)| !k.to_lowercase().contains("adslot"))
                .find_map(|(_, v)| find_video(v))
        }
        Value::Array(array) => array.iter().find_map(find_video),
        _ => None,
    }
}

fn extract_video(html: &str) -> Option<String> {
    for marker in [
        "var ytInitialData = ",
        "ytInitialData = ",
        "window[\"ytInitialData\"] = ",
    ] {
        if let Some((_, tail)) = html.split_once(marker) {
            let mut stream = serde_json::Deserializer::from_str(tail).into_iter::<Value>();
            if let Some(Ok(value)) = stream.next() {
                return find_video(&value);
            }
        }
    }
    None
}

pub fn youtube(query: &str) -> Result<String> {
    let search = youtube_search_url(query);
    let first = (|| -> Result<Option<String>> {
        let client = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(15))
            .build()?;
        let html = client
            .get(&search)
            .header("User-Agent", "Mozilla/5.0")
            .send()?
            .error_for_status()?
            .text()?;
        Ok(extract_video(&html))
    })()
    .unwrap_or(None);
    if let Some(id) = first {
        open_external(&format!("https://www.youtube.com/watch?v={id}&autoplay=1"))?;
        Ok("Открыто первое видео YouTube. Браузер может запросить запуск вручную.".into())
    } else {
        open_external(&search)?;
        Ok("Открыт поиск YouTube. Автовыбор недоступен: выберите видео в браузере.".into())
    }
}

pub trait MusicService {
    fn play(&mut self, settings: &Settings, query: &str, category: &Category) -> Result<String>;
}

#[derive(Default)]
pub struct BrowserProcess {
    pub closing: bool,
    pub child: Option<Child>,
}

impl BrowserProcess {
    pub fn close(&mut self) {
        self.closing = true;
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

pub type ProcessRegistry = Arc<Mutex<BrowserProcess>>;

#[derive(Default)]
pub struct BrowserMusic {
    chrome: Option<Chrome>,
    browser: String,
    process: ProcessRegistry,
}

impl BrowserMusic {
    pub fn with_process(process: ProcessRegistry) -> Self {
        Self {
            process,
            ..Self::default()
        }
    }
    fn chrome(&mut self, settings: &Settings) -> Result<&mut Chrome> {
        // Closing a tab can destroy its CDP session while the browser itself stays alive.
        let unavailable = self
            .chrome
            .as_mut()
            .is_some_and(|c| !c.alive() || c.evaluate("1").is_err());
        if self.browser != settings.browser || unavailable {
            self.chrome = None;
        }
        if self.chrome.is_none() {
            self.chrome = Some(Chrome::launch(&settings.browser, self.process.clone())?);
            self.browser = settings.browser.clone();
        }
        Ok(self.chrome.as_mut().unwrap())
    }

    pub fn login(&mut self, settings: &Settings) -> Result<String> {
        let url = match settings.music_service.as_str() {
            "spotify" => "https://open.spotify.com",
            "youtube" => "https://music.youtube.com",
            _ => "https://music.yandex.ru",
        };
        self.chrome(settings)?.navigate(url)?;
        Ok(
            "Войдите в сервис в открытом окне. Отдельный профиль сохраняется на этом компьютере."
                .into(),
        )
    }
}

impl MusicService for BrowserMusic {
    fn play(&mut self, settings: &Settings, query: &str, category: &Category) -> Result<String> {
        if settings.music_service == "youtube" {
            return youtube(&format!(
                "{query} {}",
                if *category == Category::Playlist {
                    "playlist"
                } else {
                    "music"
                }
            ));
        }
        let mut url = if settings.music_service == "spotify" {
            let mut url = url::Url::parse("https://open.spotify.com/search/")?;
            url.path_segments_mut().unwrap().pop_if_empty().push(query);
            url
        } else {
            let mut url = url::Url::parse("https://music.yandex.ru/search")?;
            let kind = match category {
                Category::Track => "track",
                Category::Artist => "artist",
                Category::Playlist => "playlist",
            };
            url.query_pairs_mut()
                .append_pair("text", query)
                .append_pair("type", kind);
            url
        };
        url.set_fragment(None);
        let chrome = self.chrome(settings)?;
        chrome.navigate(url.as_str())?;
        let category = match category {
            Category::Track => "track",
            Category::Artist => "artist",
            Category::Playlist => "playlist",
        };
        let script = PLAY_SCRIPT.replace("__CATEGORY__", category);
        for _ in 0..8 {
            let result = chrome.evaluate(&script)?;
            if result.as_str() == Some("playing") {
                return Ok(format!("Воспроизведение началось: {query}"));
            }
            std::thread::sleep(Duration::from_millis(600));
        }
        Ok(format!("Открыт музыкальный поиск: {query}. Если музыка не началась, войдите в аккаунт и нажмите «Играть». Возможны ограничения подписки или изменения страницы."))
    }
}

// Only fixed adapter code is evaluated; no transcribed text is interpolated into JavaScript.
const PLAY_SCRIPT: &str = r#"(() => {
  if (window.__vasyaClicked && ([...document.querySelectorAll('audio,video')].some(m => !m.paused && m.currentTime > 0)
      || navigator.mediaSession?.playbackState === 'playing')) return 'playing';
  if (window.__vasyaClicked) return 'waiting';
  const type = '__CATEGORY__';
  const roots = type === 'track'
    ? '[class*="Track_root"], .d-track, [data-testid="tracklist-row"]'
    : type === 'artist' ? '[class*="ArtistCard"], .artist, main header'
    : '[class*="PlaylistCard"], .playlist, main header';
  const candidates = [...document.querySelectorAll(roots)].flatMap(root => [...root.querySelectorAll('button')]);
  const play = candidates.find(b => /воспроизвести|играть|слушать|^play/i.test(b.getAttribute('aria-label') || b.title || b.textContent || ''))
    || document.querySelector('.d-track__start-column .d-track__play')
    || document.querySelector('button[data-testid="play-button"]');
  if (play && !play.disabled) { play.click(); window.__vasyaClicked = true; return 'clicked'; }
  if (type !== 'track' && !window.__vasyaNavigated) {
    const link = [...document.querySelectorAll(roots)].flatMap(r => [...r.querySelectorAll('a[href]')])
      .find(a => type === 'artist' ? /\/artist\//.test(a.pathname) : /\/playlists\//.test(a.pathname));
    if (link) { window.__vasyaNavigated = true; link.click(); }
  }
  return 'waiting';
})()"#;

struct Chrome {
    process: ProcessRegistry,
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    session: String,
    sequence: u64,
}

fn browser_executable(browser: &str) -> Result<PathBuf> {
    let suffix = if browser == "chrome" {
        "Google/Chrome/Application/chrome.exe"
    } else {
        "Microsoft/Edge/Application/msedge.exe"
    };
    for key in ["PROGRAMFILES", "PROGRAMFILES(X86)", "LOCALAPPDATA"] {
        if let Some(root) = std::env::var_os(key) {
            let path = PathBuf::from(root).join(suffix);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    bail!("Не найден Edge/Chrome. Установите браузер или измените выбор в настройках")
}

impl Chrome {
    fn launch(browser: &str, registry: ProcessRegistry) -> Result<Self> {
        let executable = browser_executable(browser)?;
        let profile = crate::config::data_dir().join("browser").join(browser);
        std::fs::create_dir_all(&profile)?;
        let port_file = profile.join("DevToolsActivePort");
        if port_file.exists() {
            std::fs::remove_file(&port_file)?;
        }
        let mut command = Command::new(executable);
        command
            .arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--remote-debugging-address=127.0.0.1",
                "--remote-debugging-port=0",
                "--no-first-run",
                "--no-default-browser-check",
                "about:blank",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        {
            let mut owner = registry.lock().unwrap();
            if owner.closing {
                bail!("Помощник завершает работу");
            }
            owner.child = Some(command.spawn()?);
        }
        let result = (|| -> Result<(WebSocket<MaybeTlsStream<TcpStream>>, String)> {
            let start = Instant::now();
            let port = loop {
                if let Ok(content) = std::fs::read_to_string(&port_file) {
                    if let Some(port) = content.lines().next().and_then(|s| s.parse::<u16>().ok()) {
                        break port;
                    }
                }
                let alive = registry
                    .lock()
                    .unwrap()
                    .child
                    .as_mut()
                    .is_some_and(|process| process.try_wait().ok().flatten().is_none());
                if !alive || start.elapsed() > Duration::from_secs(15) {
                    bail!("Браузер не предоставил локальное подключение. Закройте ранее открытый музыкальный профиль и повторите.");
                }
                std::thread::sleep(Duration::from_millis(100));
            };
            let client = reqwest::blocking::Client::builder()
                .no_proxy()
                .timeout(Duration::from_secs(5))
                .build()?;
            let info: Value = client
                .get(format!("http://127.0.0.1:{port}/json/version"))
                .send()?
                .json()?;
            let endpoint = info["webSocketDebuggerUrl"]
                .as_str()
                .context("Нет адреса CDP")?;
            let (mut socket, _) = tungstenite::connect(endpoint)?;
            if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
                stream.set_read_timeout(Some(Duration::from_secs(8)))?;
                stream.set_write_timeout(Some(Duration::from_secs(8)))?;
            }
            let targets = rpc(&mut socket, 1, "Target.getTargets", json!({}), None)?;
            let existing = targets["targetInfos"]
                .as_array()
                .and_then(|targets| targets.iter().find(|target| target["type"] == "page"))
                .map(|target| target["targetId"].clone());
            let target_id = match existing {
                Some(id) => id,
                None => rpc(
                    &mut socket,
                    2,
                    "Target.createTarget",
                    json!({"url":"about:blank"}),
                    None,
                )?["targetId"]
                    .clone(),
            };
            let attached = rpc(
                &mut socket,
                3,
                "Target.attachToTarget",
                json!({"targetId":target_id,"flatten":true}),
                None,
            )?;
            Ok((
                socket,
                attached["sessionId"]
                    .as_str()
                    .context("Нет сессии браузера")?
                    .into(),
            ))
        })();
        match result {
            Ok((socket, session)) => Ok(Self {
                process: registry,
                socket,
                session,
                sequence: 4,
            }),
            Err(err) => {
                if let Some(mut process) = registry.lock().unwrap().child.take() {
                    let _ = process.kill();
                    let _ = process.wait();
                }
                Err(err)
            }
        }
    }
    fn alive(&mut self) -> bool {
        self.process
            .lock()
            .unwrap()
            .child
            .as_mut()
            .is_some_and(|process| process.try_wait().ok().flatten().is_none())
    }
    fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        rpc(
            &mut self.socket,
            self.sequence,
            method,
            params,
            Some(&self.session),
        )
    }
    fn navigate(&mut self, url: &str) -> Result<()> {
        // Reset SPA flags even if navigation does not create a new JS context.
        let _ = self.evaluate("window.__vasyaClicked = false; window.__vasyaNavigated = false");
        let result = self.call("Page.navigate", json!({"url":url}))?;
        if let Some(error) = result["errorText"].as_str() {
            bail!("Браузер: {error}");
        }
        self.call("Page.bringToFront", json!({}))?;
        Ok(())
    }
    fn evaluate(&mut self, expression: &str) -> Result<Value> {
        let result = self.call(
            "Runtime.evaluate",
            json!({"expression":expression,"returnByValue":true,"userGesture":true}),
        )?;
        Ok(result["result"]["value"].clone())
    }
}

fn rpc(
    socket: &mut WebSocket<MaybeTlsStream<TcpStream>>,
    id: u64,
    method: &str,
    params: Value,
    session: Option<&str>,
) -> Result<Value> {
    let mut message = json!({"id":id,"method":method,"params":params});
    if let Some(session) = session {
        message["sessionId"] = session.into();
    }
    socket.send(Message::Text(message.to_string().into()))?;
    let start = Instant::now();
    loop {
        if start.elapsed() > Duration::from_secs(10) {
            bail!("Браузер не ответил вовремя");
        }
        let message = socket
            .read()
            .context("Музыкальный браузер закрыт или недоступен")?;
        if let Message::Text(text) = message {
            let value: Value = serde_json::from_str(&text)?;
            if value["id"].as_u64() == Some(id) {
                if !value["error"].is_null() {
                    bail!("Ошибка браузера: {}", value["error"]);
                }
                return Ok(value["result"].clone());
            }
        }
    }
}

impl Drop for Chrome {
    fn drop(&mut self) {
        let _ = rpc(
            &mut self.socket,
            self.sequence + 1,
            "Browser.close",
            json!({}),
            None,
        );
        if let Some(mut process) = self.process.lock().unwrap().child.take() {
            if process.try_wait().ok().flatten().is_none() {
                let _ = process.kill();
            }
            let _ = process.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn urls_encode_queries_as_data() {
        let url = url::Url::parse(&search_url("AC/DC & Кино #1")).unwrap();
        assert_eq!(url.query_pairs().next().unwrap().1, "AC/DC & Кино #1");
        assert_eq!(url.host_str(), Some("www.google.com"));
    }
    #[test]
    fn youtube_result_extraction_and_unavailable_results() {
        let html = r#"<script>var ytInitialData = {"contents":[{"videoRenderer":{"videoId":"abcdefgh_12"}}]};</script>"#;
        assert_eq!(extract_video(html).as_deref(), Some("abcdefgh_12"));
        assert!(extract_video("consent needed").is_none());
        assert!(find_video(&json!({"videoRenderer":{"videoId":"javascript:alert(1)"}})).is_none());
    }
}
