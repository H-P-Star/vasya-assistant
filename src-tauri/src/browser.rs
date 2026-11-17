use crate::{commands::Category, config::Settings};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    net::{TcpListener, TcpStream},
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
    pub keep_alive: bool,
}

impl BrowserProcess {
    pub fn close(&mut self) {
        self.closing = true;
        if let Some(mut child) = self.child.take() {
            if !self.keep_alive {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

pub type ProcessRegistry = Arc<Mutex<BrowserProcess>>;

#[derive(Default)]
pub struct BrowserMusic {
    chrome: Option<Chrome>,
    browser: String,
    process: ProcessRegistry,
    native_key: String,
    native_chrome: Option<Chrome>,
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
        if settings.music_mode == "desktop" && settings.music_service != "youtube" {
            self.play_native(settings, "", &Category::Track)?;
            return Ok("Музыкальное приложение открыто. Войдите в аккаунт, если требуется.".into());
        }
        self.native_chrome = None;
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
        if settings.music_mode == "desktop" {
            return self.play_native(settings, query, category);
        }
        self.native_chrome = None;
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
        if query.is_empty() {
            chrome.navigate(if settings.music_service == "spotify" {
                "https://open.spotify.com"
            } else {
                "https://music.yandex.ru"
            })?;
            return Ok("Музыкальный сервис открыт".into());
        }
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

pub fn native_music_uri(service: &str, query: &str, category: &Category) -> String {
    if service == "spotify" {
        if query.is_empty() {
            return "spotify:".into();
        }
        let encoded: String = url::form_urlencoded::byte_serialize(query.as_bytes()).collect();
        return format!("spotify:search:{}", encoded.replace('+', "%20"));
    }
    if query.is_empty() {
        return "yandexmusic://".into();
    }
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    params.append_pair("text", query).append_pair(
        "type",
        match category {
            Category::Track => "track",
            Category::Artist => "artist",
            Category::Playlist => "playlist",
        },
    );
    format!("yandexmusic://search?{}", params.finish())
}

fn native_executable(settings: &Settings) -> Result<Option<PathBuf>> {
    let (custom, candidates) = if settings.music_service == "spotify" {
        (
            &settings.spotify_app_path,
            vec![
                ("APPDATA", "Spotify/Spotify.exe"),
                ("LOCALAPPDATA", "Microsoft/WindowsApps/Spotify.exe"),
            ],
        )
    } else {
        (
            &settings.yandex_app_path,
            vec![
                ("LOCALAPPDATA", "Programs/YandexMusic/Яндекс Музыка.exe"),
                ("LOCALAPPDATA", "Programs/YandexMusic/Yandex Music.exe"),
            ],
        )
    };
    if !custom.trim().is_empty() {
        let path = PathBuf::from(custom.trim().trim_matches('"'));
        if !path.is_absolute()
            || !path.is_file()
            || !path
                .extension()
                .and_then(|s| s.to_str())
                .is_some_and(|s| s.eq_ignore_ascii_case("exe"))
        {
            bail!(
                "Не найден .exe музыкального приложения: {}. Проверьте путь в настройках.",
                path.display()
            );
        }
        return Ok(Some(path));
    }
    Ok(candidates.into_iter().find_map(|(key, suffix)| {
        std::env::var_os(key)
            .map(PathBuf::from)
            .map(|p| p.join(suffix))
            .filter(|p| p.is_file())
    }))
}

fn open_native(settings: &Settings, uri: &str) -> Result<()> {
    if let Some(exe) = native_executable(settings)? {
        Command::new(exe)
            .arg(uri)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        return Ok(());
    }
    #[cfg(windows)]
    return crate::platform::open_music_uri(uri);
    #[cfg(not(windows))]
    bail!("Музыкальное приложение поддерживается в Windows");
}

impl BrowserMusic {
    fn play_native(
        &mut self,
        settings: &Settings,
        query: &str,
        category: &Category,
    ) -> Result<String> {
        self.chrome = None;
        let uri = native_music_uri(&settings.music_service, query, category);
        let key = format!(
            "{}|{}|{}",
            settings.music_service, settings.yandex_app_path, settings.spotify_app_path
        );
        let stale = self
            .native_chrome
            .as_mut()
            .is_some_and(|c| c.evaluate("1").is_err());
        if key != self.native_key || stale {
            self.native_chrome = None;
        }
        if settings.music_service == "spotify" {
            open_native(settings, &uri)?;
            return Ok(if query.is_empty() {
                "Музыкальное приложение открыто".into()
            } else {
                format!("Открыт поиск Spotify в приложении: {query}. Выберите результат и нажмите «Играть».")
            });
        }
        if let Some(chrome) = self.native_chrome.as_mut() {
            let _ =
                chrome.evaluate("window.__vasyaClicked = false; window.__vasyaNavigated = false; window.__vasyaFiltered = false");
            open_native(settings, &uri)?;
        } else {
            // Electron accepts a loopback CDP port only at its first launch. An already
            // running client receives the deep link but may require manual playback.
            if let Some(exe) = native_executable(settings)? {
                match Chrome::launch_native(&exe, &uri, self.process.clone()) {
                    Ok(chrome) => self.native_chrome = Some(chrome),
                    Err(_) => open_native(settings, &uri)?,
                }
                self.native_key = key;
            } else {
                open_native(settings, &uri)?;
            }
        }
        if query.is_empty() {
            return Ok("Музыкальное приложение открыто".into());
        }
        if let Some(chrome) = self.native_chrome.as_mut() {
            // Deep links navigate asynchronously; do not click the previous search results.
            let mut at_search = false;
            let kind = match category {
                Category::Track => "track",
                Category::Artist => "artist",
                Category::Playlist => "playlist",
            };
            let script = PLAY_SCRIPT.replace("__CATEGORY__", kind);
            for _ in 0..12 {
                if !at_search {
                    at_search = chrome
                        .evaluate("location.href")
                        .ok()
                        .and_then(|v| v.as_str().and_then(|s| url::Url::parse(s).ok()))
                        .is_some_and(|url| {
                            url.query_pairs().any(|(k, v)| k == "text" && v == query)
                        });
                    // Give the SPA a rendering turn after the deep link changes its URL.
                    std::thread::sleep(Duration::from_millis(500));
                }
                if at_search {
                    match chrome
                        .evaluate(&script)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .as_deref()
                    {
                        Some("playing") => {
                            return Ok(format!("Воспроизведение в приложении началось: {query}"))
                        }
                        None => break,
                        _ => {}
                    }
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        }
        Ok(format!("Открыт поиск Яндекс Музыки в приложении: {query}. Если музыка не началась, войдите в аккаунт и нажмите «Играть». Для автоматического запуска попробуйте закрыть клиент через его трей и повторить команду."))
    }
}

// Only fixed adapter code is evaluated; no transcribed text is interpolated into JavaScript.
const PLAY_SCRIPT: &str = r#"(() => {
  const media = [...document.querySelectorAll('audio,video')];
  const playing = media.some(m => !m.paused && m.currentTime > 0) || navigator.mediaSession?.playbackState === 'playing';
  const metadata = navigator.mediaSession?.metadata;
  const signature = [metadata?.title, metadata?.artist, metadata?.album, ...media.map(m => m.currentSrc)].join('|');
  const time = Math.max(0, ...media.map(m => m.currentTime));
  if (window.__vasyaClicked && playing && (!window.__vasyaBefore?.playing
      || signature !== window.__vasyaBefore.signature || time + 0.5 < window.__vasyaBefore.time)) return 'playing';
  if (window.__vasyaClicked) return 'waiting';
  const type = '__CATEGORY__';
  if (!window.__vasyaFiltered) {
    const label = {track: /^(треки|песни|tracks|songs)$/i, artist: /^(исполнители|артисты|artists)$/i, playlist: /^(плейлисты|playlists)$/i}[type];
    const tab = [...document.querySelectorAll('[role="tab"], [class*="SearchPage_tab"]')]
      .find(t => label.test((t.textContent || '').trim()));
    window.__vasyaFiltered = true;
    if (tab && tab.getAttribute('aria-selected') !== 'true' && !tab.className.includes('tab_selected')) {
      tab.click(); return 'waiting';
    }
  }
  const roots = type === 'track'
    ? '[class*="Track_root"], [class*="HorizontalCardContainer_root"]:has(a[href*="/track/"]), .d-track, [data-testid="tracklist-row"]'
    : type === 'artist' ? '[class*="ArtistCard"], .artist, main header'
    : '[class*="PlaylistCard"], .playlist, main header';
  const candidates = [...document.querySelectorAll(roots)].flatMap(root => [...root.querySelectorAll('button')]);
  const play = candidates.find(b => /воспроизвести|играть|слушать|^play/i.test(b.getAttribute('aria-label') || b.title || b.textContent || ''))
    || document.querySelector('.d-track__start-column .d-track__play')
    || document.querySelector('button[data-testid="play-button"]');
  if (play && !play.disabled) {
    window.__vasyaBefore = {playing, signature, time};
    play.click(); window.__vasyaClicked = true; return 'clicked';
  }
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
    native: bool,
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
    fn launch_native(
        executable: &std::path::Path,
        uri: &str,
        registry: ProcessRegistry,
    ) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        drop(listener);
        {
            let mut owner = registry.lock().unwrap();
            if owner.closing {
                bail!("Помощник завершает работу");
            }
            owner.child = Some(
                Command::new(executable)
                    .arg("--remote-debugging-address=127.0.0.1")
                    .arg(format!("--remote-debugging-port={port}"))
                    .arg(uri)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn()?,
            );
            owner.keep_alive = true;
        }
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(500))
            .build()?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(8) {
            if registry.lock().unwrap().closing {
                bail!("Помощник завершает работу");
            }
            let info = client
                .get(format!("http://127.0.0.1:{port}/json/version"))
                .send()
                .and_then(|r| r.json::<Value>());
            if let Ok(info) = info {
                if let Some(endpoint) = info["webSocketDebuggerUrl"].as_str() {
                    let (mut socket, _) = tungstenite::connect(endpoint)?;
                    if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
                        stream.set_read_timeout(Some(Duration::from_secs(3)))?;
                        stream.set_write_timeout(Some(Duration::from_secs(3)))?;
                    }
                    let targets = rpc(&mut socket, 1, "Target.getTargets", json!({}), None)?;
                    let page = targets["targetInfos"].as_array().and_then(|list| {
                        list.iter().find(|t| {
                            t["type"] == "page"
                                && t["url"]
                                    .as_str()
                                    .is_some_and(|u| u.starts_with("music-application:"))
                        })
                    });
                    if let Some(page) = page {
                        let attached = rpc(
                            &mut socket,
                            2,
                            "Target.attachToTarget",
                            json!({"targetId":page["targetId"],"flatten":true}),
                            None,
                        )?;
                        return Ok(Self {
                            process: registry,
                            socket,
                            session: attached["sessionId"]
                                .as_str()
                                .context("Нет сессии приложения")?
                                .into(),
                            sequence: 3,
                            native: true,
                        });
                    }
                }
            }
            let exited = registry
                .lock()
                .unwrap()
                .child
                .as_mut()
                .is_none_or(|p| p.try_wait().ok().flatten().is_some());
            if exited {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        bail!("Клиент не предоставил управление воспроизведением")
    }

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
            owner.keep_alive = false;
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
                native: false,
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
        let _ = self.evaluate("window.__vasyaClicked = false; window.__vasyaNavigated = false; window.__vasyaFiltered = false");
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
        if self.native {
            // Disconnect from the user's music client without stopping their music.
            let _ = self.socket.close(None);
            self.process.lock().unwrap().child.take();
            return;
        }
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
    fn native_music_links_encode_queries_and_keep_them_inside_the_client() {
        let query = "Кино & AC/DC #1? трек";
        let uri = native_music_uri("yandex", query, &Category::Playlist);
        let url = url::Url::parse(&uri).unwrap();
        assert_eq!(url.scheme(), "yandexmusic");
        assert_eq!(url.host_str(), Some("search"));
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "text").unwrap().1,
            query
        );
        assert_eq!(
            url.query_pairs().find(|(k, _)| k == "type").unwrap().1,
            "playlist"
        );
        assert!(url.fragment().is_none());
        let spotify = native_music_uri("spotify", query, &Category::Track);
        assert!(spotify.starts_with("spotify:search:"));
        assert!(!spotify.contains("#1?"));
        assert_eq!(
            native_music_uri("yandex", "", &Category::Track),
            "yandexmusic://"
        );
    }
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
