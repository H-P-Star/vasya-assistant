use crate::{commands::Category, config::Settings};
use anyhow::{bail, Context, Result};
use serde::Deserialize;
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
    fn play_with_cancel(
        &mut self,
        settings: &Settings,
        query: &str,
        category: &Category,
        _: &dyn Fn() -> bool,
    ) -> Result<String> {
        self.play(settings, query, category)
    }
}

#[derive(Debug)]
pub struct MusicQuery {
    pub title: String,
    pub artist: String,
    pub search: String,
}
impl MusicQuery {
    pub fn parse(query: &str, category: &Category) -> Self {
        let query = query.trim().trim_matches(['«', '»', '"']);
        if matches!(category, Category::Track | Category::Auto) {
            for (pattern, min_words) in [
                (
                    r"(?i)^(.+?)\s+(?:исполнителя|исполнитель|группы|от)\s+(.+)$",
                    1,
                ),
                (r"(?i)^(.+?)\s+by\s+(.+)$", 2),
            ] {
                let pattern = regex::Regex::new(pattern).unwrap();
                if let Some(parts) = pattern.captures(query) {
                    let title = parts[1].trim().trim_matches(['«', '»', '"']).to_string();
                    let artist = parts[2].trim().trim_matches(['«', '»', '"']).to_string();
                    if artist.split_whitespace().count() < min_words {
                        continue;
                    }
                    return Self {
                        search: format!("{title} {artist}"),
                        title,
                        artist,
                    };
                }
            }
        }
        Self {
            title: query.into(),
            artist: String::new(),
            search: query.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct MusicCandidate {
    id: String,
    title: String,
    artist: String,
    kind: String,
}

fn comparable(text: &str) -> String {
    crate::commands::normalize(text)
        .chars()
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn consonants(text: &str) -> String {
    let mut text = comparable(text);
    for (from, to) in [
        ("shch", "щ"),
        ("sch", "щ"),
        ("sh", "ш"),
        ("ch", "ч"),
        ("th", "т"),
        ("ph", "ф"),
        ("ck", "к"),
        ("qu", "к"),
        ("ng", "нг"),
    ] {
        text = text.replace(from, to);
    }
    let mut result = String::new();
    for c in text.chars() {
        let c = match c {
            'b' => 'б',
            'c' | 'k' | 'q' => 'к',
            'd' => 'д',
            'f' => 'ф',
            'g' => 'г',
            'h' => 'х',
            'j' => 'ж',
            'l' => 'л',
            'm' => 'м',
            'n' => 'н',
            'p' => 'п',
            'r' => 'р',
            's' => 'с',
            't' => 'т',
            'v' | 'w' => 'в',
            'x' => 'к',
            'z' => 'з',
            c => c,
        };
        if !"aeiouyаеёиоуыэюяьъ".contains(c) && !result.ends_with(c) {
            result.push(c);
        }
    }
    result
}

fn distance(a: &str, b: &str) -> usize {
    let b: Vec<_> = b.chars().collect();
    let mut row: Vec<_> = (0..=b.len()).collect();
    for (i, a) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, b) in b.iter().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j + 1] + 1)
                .min(row[j] + 1)
                .min(previous + usize::from(a != *b));
            previous = old;
        }
    }
    row[b.len()]
}

fn phonetic_match(a: &str, b: &str) -> bool {
    let a = consonants(a);
    let b = consonants(b);
    let size = a.chars().count().max(b.chars().count());
    if size > 80 {
        return false;
    }
    a.chars().count() >= 2
        && b.chars().count() >= 2
        && distance(&a, &b) <= if size >= 8 { 2 } else { 1 }
}
fn select_candidate<'a>(
    request: &MusicQuery,
    category: &Category,
    rows: &'a [MusicCandidate],
) -> Option<&'a MusicCandidate> {
    let kind = match category {
        Category::Track | Category::Auto => "track",
        Category::Artist => "artist",
        Category::Playlist => "playlist",
    };
    let title = comparable(&request.title);
    let artist = comparable(&request.artist);
    rows.iter()
        .filter(|r| {
            !r.title.is_empty()
                && (r.kind == kind || (*category == Category::Auto && r.kind == "artist"))
        })
        .filter_map(|r| {
            let actual_title = comparable(&r.title);
            let actual_artist = comparable(&r.artist);
            if r.kind == "artist" && *category == Category::Auto {
                return (artist.is_empty() && actual_title == title).then_some((38, r));
            }
            let artist_score = if artist.is_empty() {
                0
            } else if actual_artist.contains(&artist) {
                30
            } else if artist.chars().count().min(actual_artist.chars().count()) >= 6
                && phonetic_match(&request.artist, &r.artist)
            {
                15
            } else {
                return None;
            };
            let title_score = if actual_title == title {
                40
            } else if artist.is_empty()
                && !actual_artist.is_empty()
                && title
                    .strip_prefix(&actual_artist)
                    .or_else(|| title.strip_suffix(&actual_artist))
                    .is_some_and(|part| !part.is_empty() && actual_title.contains(part))
            {
                45
            } else if actual_title.contains(&title) {
                30
            } else if !artist.is_empty()
                && artist_score >= 15
                && phonetic_match(&request.title, &r.title)
            {
                10
            } else {
                return None;
            };
            Some((title_score + artist_score, r))
        })
        .reduce(|best, next| if next.0 > best.0 { next } else { best })
        .map(|(_, r)| r)
}

fn playback_matches(value: &Value, candidate: &MusicCandidate) -> bool {
    if value["playing"] != true {
        return false;
    }
    if candidate.kind == "artist"
        && value["artist"]
            .as_str()
            .is_some_and(|a| comparable(a) == comparable(&candidate.title))
    {
        return true;
    }
    if candidate.kind != "track" || value["title"].as_str().unwrap_or_default().is_empty() {
        // Artist/playlist buttons belong to a playback context, whose current track
        // can have different metadata. The same control also covers missing MediaSession.
        return value["selectedPlaying"] == candidate.id;
    }
    let title = comparable(value["title"].as_str().unwrap_or_default());
    let artist = comparable(value["artist"].as_str().unwrap_or_default());
    !title.is_empty()
        && title == comparable(&candidate.title)
        && (candidate.artist.is_empty() || artist.contains(&comparable(&candidate.artist)))
}

// Only fixed code is evaluated. Search text and selection IDs travel as CDP data arguments.
const NATIVE_SNAPSHOT: &str = include_str!("music/results.js");

const NATIVE_SELECT_TAB: &str = r#"function(kind) {
 const names={track:/^(треки|tracks)$/i,artist:/^(исполнители|artists)$/i,playlist:/^(плейлисты|playlists)$/i};
 const tab=[...document.querySelectorAll('[role="tab"], button[class*="SearchPage_tab"]')]
   .find(el=>names[kind].test(el.textContent.trim()));
 if(!tab) return false;
 if(tab.getAttribute('aria-selected')!=='true'&&!tab.className.includes('SearchPage_tab_selected')) tab.click();
 return true;
}"#;

const NATIVE_CLICK: &str = include_str!("music/activate.js");

// Clicking is a separate step from confirmation. Repeated polls must never toggle pause.
const START_MUSIC: &str = r#"(() => {
 const playing = navigator.mediaSession?.playbackState === 'playing'
   || [...document.querySelectorAll('audio,video')].some(m => !m.paused && !m.ended && m.readyState >= 2);
 if (playing) return 'playing';
 if (window.__vasyaClicked) return 'waiting';
 const controls = document.querySelector('[class*="PlayerBarDesktop_root"], [data-testid="now-playing-bar"], ytmusic-player-bar');
 const candidates = [...(controls?.querySelectorAll('button,[role="button"]') || [])];
 const label = b => [b.getAttribute('aria-label'),b.title].filter(Boolean).join(' ');
 const available = b => !b.disabled && b.getAttribute('aria-disabled') !== 'true';
 const resume = candidates.find(b => available(b) && /^(воспроизвести|играть|продолжить|play|resume)(?:\s|$)/i.test(label(b)));
 const wave = [...document.querySelectorAll('button[class*="VibeBlock_playButton"], [class*="VibeBlock_playButton"] button')].find(available);
 // On a fresh client the player bar can show a play button with an empty queue.
 const queued = navigator.mediaSession?.metadata?.title || controls?.querySelector('a[href*="/track/"], [class*="Meta_title__"]');
 const play = (queued && resume) || wave || resume;
 if (!play) return 'waiting';
 window.__vasyaClicked = true;
 play.click();
 return 'waiting';
})()"#;

fn start_music(chrome: &mut Chrome, home: &str, cancelled: &dyn Fn() -> bool) -> Result<String> {
    chrome.evaluate("window.__vasyaClicked = false")?;
    for attempt in 0..32 {
        if cancelled() {
            bail!("Команда отменена");
        }
        if chrome.evaluate(START_MUSIC)?.as_str() == Some("playing") {
            return Ok("Музыка играет".into());
        }
        if attempt == 2
            && chrome.evaluate("Boolean(window.__vasyaClicked)")? != true
            && chrome.evaluate("location.href")?.as_str() != Some(home)
        {
            chrome.navigate(home)?;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    bail!("Плеер не подтвердил воспроизведение. Проверьте вход в аккаунт и доступность музыки.")
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
        if settings.music_mode == "desktop" {
            match settings.music_service.as_str() {
                "yandex" => {
                    self.chrome = None;
                    if let Ok(chrome) = self.connect_yandex(settings) {
                        chrome.call("Page.bringToFront", json!({}))?;
                        return Ok("Яндекс Музыка подключена".into());
                    }
                    open_native(settings, &native_music_uri("yandex", "", &Category::Track))?;
                    return Ok("Яндекс Музыка открыта. Чтобы включить управление, нажмите «Перезапустить и подключить Яндекс Музыку».".into());
                }
                "spotify" => {
                    self.native_chrome = None;
                    open_native(settings, &native_music_uri("spotify", "", &Category::Track))?;
                    return Ok("Spotify открыт. Войдите в аккаунт, если требуется.".into());
                }
                _ => {}
            }
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

    pub fn reconnect_yandex(&mut self, settings: &Settings) -> Result<String> {
        self.chrome = None;
        self.native_chrome = None;
        let _ = std::fs::remove_file(crate::config::data_dir().join("yandex-cdp.json"));
        let chrome = self.connect_yandex(settings).context(
            "Яндекс Музыка после перезапуска не предоставила управление воспроизведением",
        )?;
        chrome.call("Page.bringToFront", json!({}))?;
        Ok("Яндекс Музыка подключена".into())
    }
}

impl MusicService for BrowserMusic {
    fn play(&mut self, settings: &Settings, query: &str, category: &Category) -> Result<String> {
        self.play_with_cancel(settings, query, category, &|| false)
    }
    fn play_with_cancel(
        &mut self,
        settings: &Settings,
        query: &str,
        category: &Category,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String> {
        if cancelled() {
            bail!("Команда отменена");
        }
        if query.trim().is_empty() {
            if settings.music_mode == "desktop" && settings.music_service == "yandex" {
                #[cfg(windows)]
                if crate::platform::resume_client("yandexmusic").unwrap_or(false) {
                    return Ok("Музыка играет".into());
                }
                self.chrome = None;
                let chrome = self.connect_yandex(settings).context(
                    "Для управления включите подключение Яндекс Музыки в настройках Васи",
                )?;
                return start_music(chrome, "music-application://desktop/", cancelled);
            }
            if settings.music_mode == "desktop" && settings.music_service == "spotify" {
                open_native(settings, "spotify:")?;
                #[cfg(windows)]
                {
                    for _ in 0..18 {
                        if cancelled() {
                            bail!("Команда отменена");
                        }
                        if crate::platform::resume_client("spotify")? {
                            return Ok("Spotify включён".into());
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                    bail!("Spotify открыт. Выберите первую запись в приложении, чтобы создать очередь воспроизведения.");
                }
                #[cfg(not(windows))]
                bail!("Управление Spotify доступно в Windows");
            }
            self.native_chrome = None;
            let home = match settings.music_service.as_str() {
                "spotify" => "https://open.spotify.com",
                "youtube" => "https://music.youtube.com",
                _ => "https://music.yandex.ru",
            };
            let chrome = self.chrome(settings)?;
            // A browser session may still belong to the previously selected service.
            if chrome.evaluate("location.origin")?.as_str() != Some(home) {
                chrome.navigate(home)?;
            }
            return start_music(chrome, home, cancelled);
        }
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
            return self.play_native(settings, query, category, cancelled);
        }
        self.native_chrome = None;
        if settings.music_service == "yandex" {
            let request = MusicQuery::parse(query, category);
            return self.chrome(settings)?.play_yandex(
                url::Url::parse("https://music.yandex.ru/")?,
                &request,
                category,
                cancelled,
            );
        }
        let mut url = url::Url::parse("https://open.spotify.com/search/")?;
        url.path_segments_mut().unwrap().pop_if_empty().push(query);
        url.set_fragment(None);
        let chrome = self.chrome(settings)?;
        chrome.navigate(url.as_str())?;
        let category = match category {
            Category::Track | Category::Auto => "track",
            Category::Artist => "artist",
            Category::Playlist => "playlist",
        };
        let script = PLAY_SCRIPT.replace("__CATEGORY__", category);
        for _ in 0..8 {
            if cancelled() {
                bail!("Команда отменена");
            }
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
            Category::Track | Category::Auto => "track",
            Category::Artist => "artist",
            Category::Playlist => "playlist",
        },
    );
    format!("yandexmusic://search?{}", params.finish())
}

pub(crate) fn native_executable(settings: &Settings) -> Result<Option<PathBuf>> {
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
    fn connect_yandex(&mut self, settings: &Settings) -> Result<&mut Chrome> {
        if self.native_key != settings.yandex_app_path
            || self
                .native_chrome
                .as_mut()
                .is_some_and(|c| c.evaluate("1").is_err())
        {
            self.native_chrome = None;
        }
        if self.native_chrome.is_none() {
            let executable = native_executable(settings)?
                .context("Укажите путь к приложению Яндекс Музыки в настройках")?;
            let chrome = Chrome::connect_saved(self.process.clone())
                .or_else(|_| Chrome::launch_native(&executable, self.process.clone()))?;
            self.native_chrome = Some(chrome);
            self.native_key.clone_from(&settings.yandex_app_path);
        }
        Ok(self.native_chrome.as_mut().unwrap())
    }

    fn play_native(
        &mut self,
        settings: &Settings,
        query: &str,
        category: &Category,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String> {
        self.chrome = None;
        let request = MusicQuery::parse(query, category);
        if settings.music_service == "spotify" {
            self.native_chrome = None;
            open_native(
                settings,
                &native_music_uri("spotify", &request.search, category),
            )?;
            return Ok(format!(
                "Поиск Spotify открыт в приложении: {}. Выберите результат и нажмите «Играть».",
                request.search
            ));
        }
        let chrome = self.connect_yandex(settings).context(
            "Для управления Яндекс Музыкой нажмите «Перезапустить и подключить Яндекс Музыку» в настройках",
        )?;
        if cancelled() {
            bail!("Команда отменена");
        }
        let page = url::Url::parse(
            chrome
                .evaluate("location.href")?
                .as_str()
                .context("Нет адреса приложения")?,
        )?;
        if page.scheme() != "music-application" || page.host_str() != Some("desktop") {
            bail!("Подключение не относится к официальному клиенту Яндекс Музыки");
        }
        chrome.play_yandex(page, &request, category, cancelled)
    }
}

impl Chrome {
    fn play_yandex(
        &mut self,
        mut page: url::Url,
        request: &MusicQuery,
        category: &Category,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String> {
        // Navigate inside the native application's origin, refreshing the search state
        // even when its SPA does not react to a second incoming deep link.
        page.set_path("/search");
        page.set_query(None);
        page.set_fragment(None);
        page.query_pairs_mut().append_pair("text", &request.search);
        self.navigate(page.as_str())?;
        self.play_yandex_results(request, category, cancelled)
    }
    fn play_yandex_results(
        &mut self,
        request: &MusicQuery,
        category: &Category,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<String> {
        let kind = match category {
            Category::Track | Category::Auto => "track",
            Category::Artist => "artist",
            Category::Playlist => "playlist",
        };
        let mut tab_selected = false;
        for _ in 0..28 {
            if cancelled() {
                bail!("Команда отменена");
            }
            let snapshot = match self.evaluate(NATIVE_SNAPSHOT) {
                Ok(value) => value,
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(400));
                    continue;
                }
            };
            if let Some(reason) = snapshot["blocked"].as_str() {
                bail!("Яндекс Музыка: {reason}");
            }
            let rows: Vec<MusicCandidate> =
                serde_json::from_value(snapshot["rows"].clone()).unwrap_or_default();
            if let Some(candidate) = select_candidate(request, category, &rows) {
                if playback_matches(&snapshot["playback"], candidate)
                    && (candidate.kind == "track"
                        || snapshot["playback"]["selectedPlaying"] == candidate.id)
                {
                    return Ok(format!("Уже играет: {}", candidate.title));
                }
                let before = snapshot["playback"].clone();
                let mut clicked = false;
                for stage in ["activate", "alternate"] {
                    let mut opened = false;
                    for _ in 0..8 {
                        if cancelled() {
                            bail!("Команда отменена");
                        }
                        let locate = |phase| json!([{"value":candidate.id},{"value":candidate.kind},{"value":candidate.title},{"value":phase}]);
                        if let Some(point) = self
                            .call_function(NATIVE_CLICK, locate("hover"))?
                            .as_object()
                        {
                            self.call(
                                "Input.dispatchMouseEvent",
                                json!({"type":"mouseMoved","x":point["x"],"y":point["y"]}),
                            )?;
                            let target = self.call_function(
                                NATIVE_CLICK,
                                locate(if opened { "activate" } else { stage }),
                            )?;
                            if let Some(action) = target["action"].as_str() {
                                if cancelled() {
                                    bail!("Команда отменена");
                                }
                                if action != "open" || !opened {
                                    let count = if action == "row" { 2 } else { 1 };
                                    for event in ["mousePressed", "mouseReleased"] {
                                        self.call("Input.dispatchMouseEvent", json!({"type":event,"x":target["x"],"y":target["y"],"button":"left","clickCount":count}))?;
                                    }
                                    if action != "open" {
                                        clicked = true;
                                        break;
                                    }
                                    opened = true;
                                }
                            }
                        }
                        std::thread::sleep(Duration::from_millis(200));
                    }
                    for _ in 0..if stage == "activate" { 10 } else { 20 } {
                        if cancelled() {
                            bail!("Команда отменена");
                        }
                        let snapshot = self.evaluate(NATIVE_SNAPSHOT)?;
                        if playback_matches(&snapshot["playback"], candidate)
                            && (candidate.kind != "artist" || snapshot["playback"] != before)
                        {
                            let label = if candidate.artist.is_empty() {
                                candidate.title.clone()
                            } else {
                                format!("{} — {}", candidate.title, candidate.artist)
                            };
                            return Ok(format!("Играет: {label}"));
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
                if !clicked {
                    bail!("Найдено «{}», но управление воспроизведением недоступно. Проверьте вход и доступность записи в Яндекс Музыке.",candidate.title);
                }
                bail!("Найдено «{}», но клиент не подтвердил воспроизведение. Проверьте вход, подписку и доступность записи в Яндекс Музыке.", candidate.title);
            } else if !tab_selected && *category != Category::Auto {
                tab_selected = self
                    .call_function(NATIVE_SELECT_TAB, json!([{"value":kind}]))
                    .ok()
                    == Some(json!(true));
            }
            std::thread::sleep(Duration::from_millis(400));
        }
        bail!("В приложении не найден подходящий результат для «{}». Уточните название и исполнителя либо проверьте вход в Яндекс Музыку.",request.search);
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
    fn call_function(&mut self, function: &str, arguments: Value) -> Result<Value> {
        let context = self.context_id()?;
        let result = self.call("Runtime.callFunctionOn",json!({"functionDeclaration":function,"arguments":arguments,"executionContextId":context,"returnByValue":true,"userGesture":true}))?;
        if !result["exceptionDetails"].is_null() {
            bail!("Клиент не смог выполнить управление воспроизведением");
        }
        Ok(result["result"]["value"].clone())
    }
    fn context_id(&mut self) -> Result<u64> {
        let tree = self.call("Page.getFrameTree", json!({}))?;
        let world = self.call(
            "Page.createIsolatedWorld",
            json!({"frameId":tree["frameTree"]["frame"]["id"],"worldName":"vasya-control"}),
        )?;
        world["executionContextId"]
            .as_u64()
            .context("Нет контекста музыкального приложения")
    }

    fn connect_saved(registry: ProcessRegistry) -> Result<Self> {
        let data: Value = serde_json::from_slice(&std::fs::read(
            crate::config::data_dir().join("yandex-cdp.json"),
        )?)?;
        let port = data["port"]
            .as_u64()
            .filter(|p| (1..=65535).contains(p))
            .context("Некорректный порт клиента")?;
        let client = reqwest::blocking::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(700))
            .build()?;
        let info: Value = client
            .get(format!("http://127.0.0.1:{port}/json/version"))
            .send()?
            .json()?;
        let endpoint = info["webSocketDebuggerUrl"].as_str().context("Нет CDP")?;
        let endpoint_url = url::Url::parse(endpoint)?;
        if endpoint_url.scheme() != "ws"
            || endpoint_url.host_str() != Some("127.0.0.1")
            || endpoint_url.port() != Some(port as u16)
        {
            bail!("Неверный адрес подключения клиента");
        }
        let (mut socket, _) = tungstenite::connect(endpoint)?;
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream.set_read_timeout(Some(Duration::from_secs(3)))?;
            stream.set_write_timeout(Some(Duration::from_secs(3)))?;
        }
        let targets = rpc(&mut socket, 1, "Target.getTargets", json!({}), None)?;
        let page = targets["targetInfos"]
            .as_array()
            .and_then(|ts| {
                ts.iter().find(|t| {
                    t["type"] == "page"
                        && t["url"]
                            .as_str()
                            .is_some_and(|s| s.starts_with("music-application://desktop/"))
                })
            })
            .context("Не найдено окно Яндекс Музыки")?;
        let attached = rpc(
            &mut socket,
            2,
            "Target.attachToTarget",
            json!({"targetId":page["targetId"],"flatten":true}),
            None,
        )?;
        registry.lock().unwrap().keep_alive = true;
        Ok(Self {
            process: registry,
            socket,
            session: attached["sessionId"]
                .as_str()
                .context("Нет сессии клиента")?
                .into(),
            sequence: 3,
            native: true,
        })
    }
    fn launch_native(executable: &std::path::Path, registry: ProcessRegistry) -> Result<Self> {
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
                        let _ = std::fs::write(
                            crate::config::data_dir().join("yandex-cdp.json"),
                            json!({"port":port}).to_string(),
                        );
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
        Self::launch_profile(Command::new(executable), profile, registry)
    }

    fn launch_profile(
        mut command: Command,
        profile: PathBuf,
        registry: ProcessRegistry,
    ) -> Result<Self> {
        std::fs::create_dir_all(&profile)?;
        let port_file = profile.join("DevToolsActivePort");
        if port_file.exists() {
            std::fs::remove_file(&port_file)?;
        }
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
        if !result["exceptionDetails"].is_null() {
            bail!("Плеер не смог прочитать состояние страницы");
        }
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
    #[cfg_attr(
        not(windows),
        ignore = "set VASYA_MUSIC_TEST_BROWSER to a Chromium executable"
    )]
    fn yandex_selected_result_starts_audio_through_real_pointer_input() -> Result<()> {
        let executable = std::env::var_os("VASYA_MUSIC_TEST_BROWSER")
            .map(PathBuf::from)
            .map(Ok)
            .unwrap_or_else(|| browser_executable("edge"))?;
        let dir = tempfile::tempdir()?;
        let mut command = Command::new(executable);
        command.args(["--headless=new", "--mute-audio"]);
        #[cfg(target_os = "linux")]
        command.arg("--no-sandbox"); // Isolated headless fixture in root-owned CI containers.
        let mut chrome =
            Chrome::launch_profile(command, dir.path().join("profile"), Default::default())?;
        let page = dir.path().join("results.html");
        std::fs::write(&page, include_str!("../tests/fixtures/music-results.html"))?;
        crate::speech::write_wav(&dir.path().join("sample.wav"), &[0; 16000])?;
        for mode in ["button", "row", "inert", "artist"] {
            let mut url = url::Url::from_file_path(&page).unwrap();
            url.set_fragment(Some(mode));
            chrome.navigate("about:blank")?;
            chrome.navigate(url.as_str())?;
            let (query, category) = if mode == "artist" {
                ("Кино", Category::Auto)
            } else {
                ("Перемен исполнителя Кино", Category::Track)
            };
            chrome
                .play_yandex_results(&MusicQuery::parse(query, &category), &category, &|| false)?;
            assert_eq!(
                chrome.evaluate("document.body.dataset.started")?,
                if mode == "artist" { "artist" } else { "track" }
            );
            assert_eq!(
                chrome.evaluate("!document.querySelector('audio').paused")?,
                true
            );
        }
        Ok(())
    }
    #[test]
    fn track_artist_query_selects_the_right_performer_and_verifies_playback() {
        let request = MusicQuery::parse("«Перемен» исполнителя Кино", &Category::Track);
        assert_eq!(request.title, "Перемен");
        assert_eq!(request.artist, "Кино");
        assert_eq!(request.search, "Перемен Кино");
        let rows = vec![
            MusicCandidate {
                id: "wrong".into(),
                title: "Перемен".into(),
                artist: "Другая группа".into(),
                kind: "track".into(),
            },
            MusicCandidate {
                id: "right".into(),
                title: "Хочу перемен!".into(),
                artist: "Кино".into(),
                kind: "track".into(),
            },
        ];
        let selected = select_candidate(&request, &Category::Track, &rows).unwrap();
        assert_eq!(selected.id, "right");
        for query in ["Кино Перемен", "Перемен Кино"] {
            let request = MusicQuery::parse(query, &Category::Auto);
            assert_eq!(
                select_candidate(&request, &Category::Auto, &rows)
                    .unwrap()
                    .id,
                "right"
            );
        }
        assert!(!playback_matches(
            &json!({"playing":true,"title":"Перемен","artist":"Другая группа"}),
            selected
        ));
        assert!(playback_matches(
            &json!({"playing":true,"title":"Хочу перемен!","artist":"Кино"}),
            selected
        ));
        let english = MusicQuery::parse("Numb by Linkin Park", &Category::Track);
        assert_eq!(english.search, "Numb Linkin Park");
        assert_eq!(
            MusicQuery::parse("Stand by Me", &Category::Track).title,
            "Stand by Me"
        );
        let explicit = MusicQuery::parse("Stand by Me исполнителя Ben E. King", &Category::Track);
        assert_eq!(explicit.title, "Stand by Me");
        assert_eq!(explicit.artist, "Ben E. King");
    }
    #[test]
    fn phonetic_titles_require_an_explicit_matching_artist() {
        let rows = vec![
            MusicCandidate {
                id: "right".into(),
                title: "Numb".into(),
                artist: "Linkin Park".into(),
                kind: "track".into(),
            },
            MusicCandidate {
                id: "wrong".into(),
                title: "Name".into(),
                artist: "Different performer".into(),
                kind: "track".into(),
            },
        ];
        let request = MusicQuery::parse("нам исполнителя Lidkin Park", &Category::Track);
        assert_eq!(
            select_candidate(&request, &Category::Track, &rows)
                .unwrap()
                .id,
            "right"
        );
        assert!(select_candidate(
            &MusicQuery::parse("нам", &Category::Track),
            &Category::Track,
            &rows
        )
        .is_none());
    }
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
