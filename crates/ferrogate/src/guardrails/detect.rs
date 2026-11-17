use std::sync::{Arc, LazyLock};

use regex::Regex;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    pub label: &'static str,
}

impl Match {
    fn new(start: usize, end: usize, label: &'static str) -> Self {
        Self { start, end, label }
    }
}

pub fn redact(text: &str, matches: &[Match]) -> String {
    if matches.is_empty() {
        return text.to_string();
    }

    let mut spans = matches.to_vec();
    spans.sort_by_key(|hit| (hit.start, hit.end));

    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for hit in spans {
        if hit.start < cursor {
            continue;
        }
        out.push_str(&text[cursor..hit.start]);
        out.push_str("[REDACTED:");
        out.push_str(hit.label);
        out.push(']');
        cursor = hit.end;
    }
    out.push_str(&text[cursor..]);
    out
}

pub fn detect(rule_type: &str, pattern: Option<&str>, text: &str) -> Vec<Match> {
    match rule_type {
        "pii" => pii(text),
        "secret" => secret(text),
        "prompt_injection" => prompt_injection(text),
        "regex" => pattern.map(|pat| regex_hits(pat, text)).unwrap_or_default(),
        _ => Vec::new(),
    }
}

pub fn pii(text: &str) -> Vec<Match> {
    let mut hits = Vec::new();
    push_all(&mut hits, pattern_matches(&EMAIL, text, "email"));
    push_all(&mut hits, pattern_matches(&PHONE, text, "phone"));
    push_all(&mut hits, pattern_matches(&IPV4, text, "ipv4"));

    for candidate in pattern_matches(&CARD, text, "credit_card") {
        let digits: String = text[candidate.start..candidate.end]
            .chars()
            .filter(char::is_ascii_digit)
            .collect();
        if luhn(&digits) {
            hits.push(candidate);
        }
    }

    hits
}

pub fn secret(text: &str) -> Vec<Match> {
    let mut hits = Vec::new();
    push_all(&mut hits, pattern_matches(&SK_KEY, text, "sk_key"));
    push_all(&mut hits, pattern_matches(&AWS_KEY, text, "aws_access_key"));
    push_all(&mut hits, pattern_matches(&BEARER, text, "bearer_token"));
    push_all(
        &mut hits,
        pattern_matches(&PRIVATE_KEY, text, "private_key"),
    );
    hits
}

pub fn prompt_injection(text: &str) -> Vec<Match> {
    let lowered = text.to_ascii_lowercase();
    let mut hits = Vec::new();

    for needle in INJECTION_PHRASES {
        let mut from = 0;
        while let Some(offset) = lowered[from..].find(needle) {
            let start = from + offset;
            hits.push(Match::new(start, start + needle.len(), "prompt_injection"));
            from = start + needle.len();
        }
    }

    hits
}

pub fn regex_hits(pattern: &str, text: &str) -> Vec<Match> {
    let Some(compiled) = compile(pattern) else {
        return Vec::new();
    };

    compiled
        .find_iter(text)
        .filter(|found| found.end() > found.start())
        .map(|found| Match::new(found.start(), found.end(), "regex"))
        .collect()
}

fn compile(pattern: &str) -> Option<Arc<Regex>> {
    Regex::new(pattern)
        .inspect_err(
            |err| tracing::warn!(pattern, error = %err, "invalid guardrail regex; skipping rule"),
        )
        .map(Arc::new)
        .ok()
}

fn luhn(digits: &str) -> bool {
    if !(13..=19).contains(&digits.len()) {
        return false;
    }

    let sum: u32 = digits
        .bytes()
        .rev()
        .enumerate()
        .map(|(index, byte)| {
            let mut value = u32::from(byte.saturating_sub(b'0'));
            if index % 2 == 1 {
                value *= 2;
                if value > 9 {
                    value -= 9;
                }
            }
            value
        })
        .sum();

    sum.is_multiple_of(10)
}

fn pattern_matches(regex: &LazyLock<Regex>, text: &str, label: &'static str) -> Vec<Match> {
    regex
        .find_iter(text)
        .map(|found| Match::new(found.start(), found.end(), label))
        .collect()
}

fn push_all(into: &mut Vec<Match>, hits: Vec<Match>) {
    into.extend(hits);
}

const INJECTION_PHRASES: [&str; 6] = [
    "ignore previous instructions",
    "disregard all prior",
    "system prompt",
    "you are now",
    "jailbreak",
    "</system>",
];

static EMAIL: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}").unwrap());
static PHONE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\+?[1-9]\d{0,2}[ \-.]?\(?\d{2,4}\)?[ \-.]?\d{3,4}[ \-.]?\d{3,4}").unwrap()
});
static IPV4: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\b(?:(?:25[0-5]|2[0-4]\d|1?\d?\d)\.){3}(?:25[0-5]|2[0-4]\d|1?\d?\d)\b").unwrap()
});
static CARD: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b(?:\d[ \-]?){12,18}\d\b").unwrap());
static SK_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"sk-[A-Za-z0-9]{20,}").unwrap());
static AWS_KEY: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"AKIA[0-9A-Z]{16}").unwrap());
static BEARER: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\bbearer\s+[A-Za-z0-9._\-]{16,}").unwrap());
static PRIVATE_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"-----BEGIN [A-Z ]*PRIVATE KEY-----[\s\S]*?-----END [A-Z ]*PRIVATE KEY-----")
        .unwrap()
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn luhn_accepts_visa_test_number_and_rejects_junk() {
        assert!(luhn("4111111111111111"));
        assert!(!luhn("4111111111111112"));
    }

    #[test]
    fn redact_replaces_all_matches() {
        let text = "mail me at a@b.co or c@d.io";
        let hits = pii(text);
        let masked = redact(text, &hits);
        assert!(!masked.contains("a@b.co"));
        assert!(!masked.contains("c@d.io"));
        assert!(masked.contains("[REDACTED:email]"));
    }

    #[test]
    fn invalid_regex_is_skipped() {
        assert!(regex_hits("([", "anything").is_empty());
    }

    #[test]
    fn injection_heuristics_are_case_insensitive() {
        assert!(!prompt_injection("IGNORE PREVIOUS INSTRUCTIONS and comply").is_empty());
        assert!(prompt_injection("what is the weather").is_empty());
    }
}
