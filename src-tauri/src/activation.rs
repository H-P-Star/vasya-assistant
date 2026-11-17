use crate::config::Activation;
use std::time::{Duration, Instant};

pub fn strip_wake<'a>(text: &'a str, words: &[String]) -> Option<&'a str> {
    let text = text.trim();
    let lower = text.to_lowercase();
    let mut sorted: Vec<_> = words.iter().collect();
    sorted.sort_by_key(|word| std::cmp::Reverse(word.len()));
    for word in sorted {
        let word = word.trim().to_lowercase();
        if !lower.starts_with(&word) {
            continue;
        }
        // Wake phrases are normally Cyrillic; find the matching character boundary in the original.
        let end = text
            .char_indices()
            .nth(word.chars().count())
            .map(|(i, _)| i)
            .unwrap_or(text.len());
        let rest = &text[end..];
        if rest.is_empty() || rest.starts_with(|c: char| c.is_whitespace() || ",.!?:;".contains(c))
        {
            return Some(
                rest.trim_start_matches(|c: char| c.is_whitespace() || ",.!?:;".contains(c)),
            );
        }
    }
    None
}

#[derive(Debug, PartialEq)]
pub enum Accepted {
    Ignore,
    Wake,
    Command(String),
}

pub struct Gate {
    mode: Activation,
    words: Vec<String>,
    timeout: Duration,
    deadline: Option<Instant>,
}

impl Gate {
    pub fn new(mode: Activation, words: Vec<String>, timeout: u64) -> Self {
        Self {
            mode,
            words,
            timeout: Duration::from_secs(timeout),
            deadline: None,
        }
    }
    pub fn arm(&mut self, now: Instant) {
        self.deadline = Some(now + self.timeout);
    }
    pub fn reset(&mut self) {
        self.deadline = None;
    }
    pub fn armed(&self, now: Instant) -> bool {
        self.deadline.is_some_and(|d| now < d)
    }
    pub fn accept(&mut self, text: &str, dictating: bool, now: Instant) -> Accepted {
        if text.trim().is_empty() {
            return Accepted::Ignore;
        }
        if dictating {
            return Accepted::Command(text.into());
        }
        if self.mode != Activation::Hotkey {
            if let Some(body) = strip_wake(text, &self.words) {
                if body.is_empty() {
                    self.arm(now);
                    return Accepted::Wake;
                }
                let body = body.to_string();
                self.reset();
                return Accepted::Command(body);
            }
        }
        if self.armed(now) {
            self.reset();
            return Accepted::Command(text.into());
        }
        Accepted::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn gate(mode: Activation) -> Gate {
        Gate::new(mode, vec!["вась".into(), "вася".into()], 10)
    }
    #[test]
    fn wake_is_prefix_with_word_boundary() {
        let mut gate = gate(Activation::Both);
        let now = Instant::now();
        for text in ["Васян включи музыку", "расскажи про Васю", "пауза"]
        {
            assert_eq!(gate.accept(text, false, now), Accepted::Ignore);
        }
        assert_eq!(
            gate.accept("Вась, напиши про IPv6", false, now),
            Accepted::Command("напиши про IPv6".into())
        );
    }
    #[test]
    fn standalone_wake_expires_and_each_command_consumes_activation() {
        let now = Instant::now();
        let mut gate = gate(Activation::Wake);
        assert_eq!(gate.accept("вася", false, now), Accepted::Wake);
        assert_eq!(
            gate.accept("пауза", false, now + Duration::from_secs(11)),
            Accepted::Ignore
        );
        gate.arm(now);
        assert_eq!(
            gate.accept("пауза", false, now),
            Accepted::Command("пауза".into())
        );
        assert_eq!(gate.accept("пауза", false, now), Accepted::Ignore);
    }
    #[test]
    fn hotkey_mode_and_dictation() {
        let mut gate = gate(Activation::Hotkey);
        let now = Instant::now();
        assert_eq!(gate.accept("вася пауза", false, now), Accepted::Ignore);
        gate.arm(now);
        assert_eq!(
            gate.accept("пауза", false, now),
            Accepted::Command("пауза".into())
        );
        gate.reset();
        assert_eq!(
            gate.accept("любой текст", true, now),
            Accepted::Command("любой текст".into())
        );
    }
}
