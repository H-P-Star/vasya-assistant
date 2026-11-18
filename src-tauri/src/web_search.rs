use anyhow::{bail, Context, Result};
use reqwest::{header, redirect::Policy, Client, Response};
use scraper::{ElementRef, Html, Selector};
use serde::Deserialize;
use std::{collections::HashSet, net::IpAddr, time::Duration};
use url::{Host, Url};

const DUCKDUCKGO_LITE: &str = "https://lite.duckduckgo.com/lite/";
const MAX_SOURCES: usize = 4;
const MAX_PAGE_BYTES: usize = 512 * 1024;

#[derive(Debug)]
pub(crate) struct Source {
    pub title: String,
    pub url: Url,
    pub snippet: String,
    pub excerpt: String,
}

// SearXNG is an explicitly configured service and may run on a private network.
pub(crate) fn searx_endpoint(base: &str) -> Result<Url> {
    let mut url = Url::parse(base.trim()).context("Укажите корректный адрес SearXNG")?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        bail!("Адрес SearXNG должен быть HTTP(S), без пароля, параметров и фрагмента");
    }
    let path = url.path().trim_end_matches('/');
    if path.ends_with("/search") {
        let path = path.to_owned();
        url.set_path(&path);
    } else {
        url.set_path(&format!("{path}/search"));
    }
    Ok(url)
}

pub(crate) fn public_url(url: &Url) -> bool {
    if !matches!(url.scheme(), "http" | "https")
        || url.port().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.as_str().len() > 2048
    {
        return false;
    }
    match url.host() {
        Some(Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            host.contains('.')
                && ![".localhost", ".local", ".internal", ".home.arpa", ".onion"]
                    .iter()
                    .any(|suffix| host.ends_with(suffix))
        }
        Some(Host::Ipv4(ip)) => public_ip(ip.into()),
        Some(Host::Ipv6(ip)) => public_ip(ip.into()),
        None => false,
    }
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(
                (a, b, c),
                (0 | 10 | 127 | 224..=255, _, _)
                    | (100, 64..=127, _)
                    | (169, 254, _)
                    | (172, 16..=31, _)
                    | (192, 0, 0 | 2)
                    | (192, 88, 99)
                    | (192, 168, _)
                    | (198, 18 | 19, _)
                    | (198, 51, 100)
                    | (203, 0, 113)
            )
        }
        IpAddr::V6(ip) => {
            let [a, b, ..] = ip.segments();
            matches!(a, 0x2000..=0x3fff)
                && !matches!((a, b), (0x2001, 0..=0x01ff | 0x0db8) | (0x2002 | 0x3fff, _))
        }
    }
}

fn compact(text: &str, limit: usize) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(limit)
        .collect()
}

fn element_text(element: ElementRef<'_>, limit: usize) -> String {
    compact(&element.text().collect::<Vec<_>>().join(" "), limit)
}

fn duckduckgo_url(href: &str, base: &Url) -> Option<Url> {
    let url = base.join(href).ok()?;
    if matches!(
        url.host_str(),
        Some("duckduckgo.com" | "lite.duckduckgo.com")
    ) && url.path() == "/l/"
    {
        let (_, value) = url.query_pairs().find(|(key, _)| key == "uddg")?;
        return Url::parse(&value).ok();
    }
    Some(url)
}

fn parse_duckduckgo(html: &str, base: &Url) -> Vec<Source> {
    let document = Html::parse_document(html);
    let selector = Selector::parse("a.result-link, td.result-snippet").unwrap();
    let mut sources: Vec<Source> = Vec::new();
    let mut current = None;
    for element in document.select(&selector) {
        if element.value().name() == "a" {
            current = None;
            let Some(url) = element
                .attr("href")
                .and_then(|href| duckduckgo_url(href, base))
                .filter(public_url)
            else {
                continue;
            };
            let title = element_text(element, 180);
            if !title.is_empty() {
                current = Some(sources.len());
                sources.push(Source {
                    title,
                    url,
                    snippet: String::new(),
                    excerpt: String::new(),
                });
            }
        } else if let Some(index) = current.take() {
            sources[index].snippet = element_text(element, 600);
        }
    }
    sources
}

#[derive(Deserialize)]
struct SearxResults {
    results: Vec<SearxResult>,
}

#[derive(Deserialize)]
struct SearxResult {
    title: String,
    url: String,
    #[serde(default)]
    content: Option<String>,
}

async fn read_limited(mut response: Response) -> Result<Vec<u8>> {
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        let left = MAX_PAGE_BYTES - body.len();
        body.extend_from_slice(&chunk[..chunk.len().min(left)]);
        if body.len() == MAX_PAGE_BYTES {
            break;
        }
    }
    Ok(body)
}

async fn search(query: &str, searx: &str) -> Result<Vec<Source>> {
    let mut url = if searx.trim().is_empty() {
        Url::parse(DUCKDUCKGO_LITE)?
    } else {
        searx_endpoint(searx)?
    };
    let is_searx = !searx.trim().is_empty();
    url.query_pairs_mut().append_pair("q", query);
    if is_searx {
        url.query_pairs_mut().append_pair("format", "json");
    }
    let client = Client::builder()
        .user_agent("Mozilla/5.0 (compatible; Vasya Assistant)")
        .redirect(Policy::none())
        .timeout(Duration::from_secs(12))
        .build()?;
    let response = client
        .get(url.clone())
        .send()
        .await
        .context("Поисковый сервис недоступен")?;
    if response.status().as_u16() == 202 && !is_searx {
        bail!(
            "DuckDuckGo запросил проверку. Повторите позже или укажите свой SearXNG в настройках"
        );
    }
    if !response.status().is_success() {
        bail!(
            "Поисковый сервис вернул HTTP {}. Для SearXNG проверьте, включена ли выдача JSON",
            response.status().as_u16()
        );
    }
    let body = read_limited(response).await?;
    let results = if is_searx {
        serde_json::from_slice::<SearxResults>(&body)
            .context("Не удалось прочитать выдачу SearXNG")?
            .results
            .into_iter()
            .filter_map(|result| {
                let url = Url::parse(&result.url).ok()?;
                Some(Source {
                    title: compact(&result.title, 180),
                    url,
                    snippet: compact(result.content.as_deref().unwrap_or_default(), 600),
                    excerpt: String::new(),
                })
            })
            .collect()
    } else {
        parse_duckduckgo(&String::from_utf8_lossy(&body), &url)
    };
    let mut seen = HashSet::new();
    let results: Vec<_> = results
        .into_iter()
        .filter_map(|mut source| {
            source.url.set_fragment(None);
            (public_url(&source.url) && seen.insert(source.url.clone())).then_some(source)
        })
        .take(MAX_SOURCES)
        .collect();
    if results.is_empty() {
        bail!("В результатах поиска нет доступных источников. Попробуйте уточнить вопрос");
    }
    Ok(results)
}

async fn download_page(mut url: Url) -> Result<Vec<u8>> {
    for _ in 0..3 {
        if !public_url(&url) {
            bail!("Недоступный адрес источника");
        }
        let mut builder = Client::builder()
            .no_proxy()
            .redirect(Policy::none())
            .user_agent("VasyaAssistant/0.4")
            .timeout(Duration::from_secs(7));
        if let Some(Host::Domain(host)) = url.host() {
            let addresses: Vec<_> = tokio::net::lookup_host((host, 0)).await?.collect();
            if addresses.is_empty() || addresses.iter().any(|addr| !public_ip(addr.ip())) {
                bail!("Источник разрешается в непубличный адрес");
            }
            // Pin the checked addresses so the HTTP connection cannot re-resolve a hostname.
            builder = builder.resolve_to_addrs(host, &addresses);
        }
        let response = builder.build()?.get(url.clone()).send().await?;
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(header::LOCATION)
                .context("Нет адреса перенаправления")?;
            url = url.join(location.to_str()?)?;
            continue;
        }
        let response = response.error_for_status()?;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !["text/html", "application/xhtml+xml", "text/plain"]
            .iter()
            .any(|kind| content_type.starts_with(kind))
        {
            bail!("Источник не содержит HTML или обычный текст");
        }
        return read_limited(response).await;
    }
    bail!("Слишком много перенаправлений страницы")
}

fn relevant_excerpt(html: &str, query: &str) -> String {
    let document = Html::parse_document(html);
    let selector = Selector::parse("h1, article p, main p, p, pre").unwrap();
    let lower = query.to_lowercase();
    let terms: Vec<_> = lower
        .split(|ch: char| !ch.is_alphanumeric())
        .filter(|word| word.chars().count() >= 4)
        .take(16)
        .collect();
    let mut passages: Vec<_> = document
        .select(&selector)
        .take(400)
        .filter_map(|element| {
            let text = element_text(element, 830);
            if text.chars().count() < 30 {
                return None;
            }
            let lower = text.to_lowercase();
            let score = terms.iter().filter(|word| lower.contains(**word)).count();
            Some((score, text))
        })
        .collect();
    passages.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    compact(
        &passages
            .into_iter()
            .take(5)
            .map(|entry| entry.1)
            .collect::<Vec<_>>()
            .join(" "),
        1600,
    )
}

pub(crate) fn sources(
    query: &str,
    searx: &str,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<Source>> {
    let query = compact(query, 500);
    if query.is_empty() {
        bail!("Скажите, что нужно узнать в интернете");
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        let mut research = Box::pin(async {
            let mut sources = search(&query, searx).await?;
            let mut downloads = tokio::task::JoinSet::new();
            for (index, source) in sources.iter().enumerate() {
                let url = source.url.clone();
                downloads.spawn(async move {
                    let page =
                        tokio::time::timeout(Duration::from_secs(9), download_page(url)).await;
                    (index, page.ok().and_then(Result::ok))
                });
            }
            while let Some(Ok((index, page))) = downloads.join_next().await {
                if let Some(page) = page {
                    sources[index].excerpt =
                        relevant_excerpt(&String::from_utf8_lossy(&page), &query);
                }
            }
            Ok(sources)
        });
        loop {
            if cancelled() {
                bail!("Веб-поиск отменён");
            }
            if let Ok(result) = tokio::time::timeout(Duration::from_millis(50), &mut research).await
            {
                return result;
            }
        }
    })
}

pub(crate) fn with_links(answer: &str, sources: &[Source]) -> String {
    let mut output = format!("{answer}\n\n### Источники\n");
    for (index, source) in sources.iter().enumerate() {
        let title = source.title.replace(['[', ']', '\\'], " ");
        let title = if title.trim().is_empty() {
            source.url.host_str().unwrap_or("Источник")
        } else {
            title.trim()
        };
        let note = if !source.excerpt.is_empty() {
            ""
        } else if source.snippet.is_empty() {
            " — только ссылка"
        } else {
            " — поисковый сниппет"
        };
        output.push_str(&format!(
            "\n- [{} · {}](<{}>){note}",
            index + 1,
            title,
            source.url
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duckduckgo_decodes_source_urls_and_keeps_their_snippets() {
        let html = r#"
        <table><tr><td><a class="result-link" href="//duckduckgo.com/l/?uddg=https%3A%2F%2Fwww.rfc-editor.org%2Frfc%2Frfc8200.html&amp;rut=123">RFC &amp; IPv6</a></td></tr>
        <tr><td class="result-snippet">IPv6 specification.</td></tr>
        <tr><td><a class="result-link" href="http://127.0.0.1/">Локальный адрес</a></td></tr>
        <tr><td class="result-snippet">Unrelated.</td></tr></table>"#;
        let sources = parse_duckduckgo(html, &Url::parse(DUCKDUCKGO_LITE).unwrap());
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].title, "RFC & IPv6");
        assert_eq!(sources[0].snippet, "IPv6 specification.");
        let card = with_links("См. [1].", &sources);
        assert!(card.contains("](<https://www.rfc-editor.org/rfc/rfc8200.html>)"));
        assert!(card.contains("поисковый сниппет"));
    }

    #[test]
    fn page_fetch_accepts_public_http_and_keeps_local_search_configuration_separate() {
        for address in [
            "file:///C:/Windows/test",
            "http://localhost./",
            "http://127.0.0.1/",
            "http://169.254.169.254/",
            "http://198.18.0.1/",
            "http://[::1]/",
            "http://[::ffff:7f00:1]/",
            "https://user:pass@example.com/",
            "https://example.com:444/",
        ] {
            assert!(!public_url(&Url::parse(address).unwrap()), "{address}");
        }
        assert!(public_url(&Url::parse("https://example.com/").unwrap()));
        assert_eq!(
            searx_endpoint("http://localhost:8080/searx/")
                .unwrap()
                .as_str(),
            "http://localhost:8080/searx/search"
        );
        assert_eq!(
            searx_endpoint("https://example.com/search/")
                .unwrap()
                .as_str(),
            "https://example.com/search"
        );
    }

    #[test]
    fn excerpts_prioritize_relevant_text_and_skip_scripts() {
        let html = r#"<main><p>Посторонний абзац со справочной информацией о сайте.</p>
        <script>IPv6 RFC 8200 посторонний код</script>
        <article><p>RFC 8200 определяет интернет-протокол IPv6.</p></article></main>"#;
        let excerpt = relevant_excerpt(html, "IPv6 RFC 8200");
        assert!(excerpt.starts_with("RFC 8200 определяет"));
        assert!(!excerpt.contains("посторонний код"));
    }
}
