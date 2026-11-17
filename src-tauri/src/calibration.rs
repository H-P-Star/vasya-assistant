use crate::{commands::normalize, config::SpeechCorrection};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct Report {
    pub wake_detected: bool,
    pub expected: String,
    pub vosk_text: String,
    pub refined_text: String,
    pub rms: f32,
    pub peak: f32,
    pub clipped_percent: f32,
    pub suggested_gain: f32,
    pub matches: bool,
    pub wake_candidate: Option<String>,
    pub message: String,
}

pub fn analyze(samples: &[i16], expected: &str, vosk_text: String, refined_text: String) -> Report {
    let size = samples.len().max(1) as f32;
    let rms = (samples
        .iter()
        .map(|s| (*s as f32 / 32768.0).powi(2))
        .sum::<f32>()
        / size)
        .sqrt();
    let peak = samples
        .iter()
        .map(|s| (*s as f32 / 32768.0).abs())
        .fold(0f32, f32::max);
    let clipped_percent =
        samples.iter().filter(|s| s.unsigned_abs() >= 32000).count() as f32 / size * 100.0;
    let matches = normalize(if refined_text.is_empty() {
        &vosk_text
    } else {
        &refined_text
    }) == normalize(expected);
    let heard = normalize(&vosk_text);
    let wake_candidate = (!expected.is_empty()
        && expected.split_whitespace().count() == 1
        && !heard.is_empty()
        && heard.split_whitespace().count() <= 2)
        .then_some(heard);
    let message = if expected.is_empty() { "Шум измерен. Теперь произнесите фразу обычным голосом." }
        else if clipped_percent > 1.0 { "Есть перегрузка. Уменьшите усиление микрофона или отодвиньте его." }
        else if peak < 0.01 { "Слишком тихо. Приблизьте микрофон или проверьте его громкость." }
        else if matches { "Фраза распознана верно." }
        else { "Сравните распознанный текст. Вариант активации или словарное исправление можно сохранить ниже." }.into();
    Report {
        wake_detected: false,
        expected: expected.into(),
        vosk_text,
        refined_text,
        rms,
        peak,
        clipped_percent,
        suggested_gain: if peak < 0.001 {
            1.0
        } else {
            (0.5 / peak).clamp(0.25, 8.0)
        },
        matches,
        wake_candidate,
        message,
    }
}

pub fn correct(text: &str, corrections: &[SpeechCorrection]) -> String {
    let mut result = text.to_string();
    for correction in corrections {
        if correction.heard.trim().is_empty() {
            continue;
        }
        let Ok(pattern) =
            regex::Regex::new(&format!("(?i){}", regex::escape(correction.heard.trim())))
        else {
            continue;
        };
        let mut next = String::new();
        let mut end = 0;
        for m in pattern.find_iter(&result) {
            let word = |c: char| c.is_alphanumeric() || c == '_';
            if result[..m.start()].chars().next_back().is_some_and(word)
                || result[m.end()..].chars().next().is_some_and(word)
            {
                continue;
            }
            next.push_str(&result[end..m.start()]);
            next.push_str(&correction.replacement);
            end = m.end();
        }
        next.push_str(&result[end..]);
        result = next;
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calibration_separates_silence_clipping_and_recognition() {
        let quiet = analyze(&[0; 1600], "", String::new(), String::new());
        assert_eq!(quiet.suggested_gain, 1.0);
        assert_eq!(quiet.rms, 0.0);
        let loud = analyze(&[32767; 1600], "вась", "вас".into(), String::new());
        assert!(loud.clipped_percent > 99.0);
        assert!(!loud.matches);
        assert_eq!(loud.wake_candidate.as_deref(), Some("вас"));
        let english = analyze(
            &[1000; 1600],
            "Linkin Park",
            "линкин парк".into(),
            "Linkin Park".into(),
        );
        assert!(english.matches);
    }
    #[test]
    fn personal_dictionary_preserves_boundaries_and_other_text() {
        let rules = vec![
            SpeechCorrection {
                heard: "линкин парк".into(),
                replacement: "Linkin Park".into(),
            },
            SpeechCorrection {
                heard: "намб".into(),
                replacement: "Numb".into(),
            },
        ];
        assert_eq!(
            correct("Включи Линкин Парк — намб!", &rules),
            "Включи Linkin Park — Numb!"
        );
        assert_eq!(correct("намбо, супернамб", &rules), "намбо, супернамб");
    }
}
