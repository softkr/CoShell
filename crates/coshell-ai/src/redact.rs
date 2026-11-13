//! Mask secrets in terminal output before it is sent to an AI provider.

use std::sync::LazyLock;

use regex::Regex;

/// Replacement shown instead of a secret.
pub const MASK: &str = "••••••••";

struct Rule {
    re: Regex,
    /// Capture group holding the secret (0 = whole match).
    group: usize,
}

static RULES: LazyLock<Vec<Rule>> = LazyLock::new(|| {
    let r = |p: &str, group| Rule { re: Regex::new(p).expect("valid redaction regex"), group };
    vec![
        // PEM private key blocks.
        r(r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?-----END [A-Z ]*PRIVATE KEY-----", 0),
        // NAME=value / NAME: value for secret-looking names.
        r(r#"(?i)\b[A-Z0-9_]*(?:PASSWORD|PASSWD|PWD|SECRET|TOKEN|API_?KEY|ACCESS_?KEY|PRIVATE_?KEY|CREDENTIALS?)[A-Z0-9_]*\s*[=:]\s*["']?([^\s"']+)"#, 1),
        // Authorization headers.
        r(r"(?i)\b(?:authorization:\s*(?:bearer|basic)|bearer)\s+([A-Za-z0-9._~+/=-]{8,})", 1),
        // Provider keys / tokens.
        r(r"\bsk-(?:ant-|proj-|live-|test-)?[A-Za-z0-9_-]{8,}", 0),
        r(r"\b(?:AKIA|ASIA)[0-9A-Z]{16}\b", 0),
        r(r"\bgh[pousr]_[A-Za-z0-9]{20,}\b", 0),
        r(r"\bxox[abprs]-[A-Za-z0-9-]{10,}\b", 0),
        // URL credentials: scheme://user:pass@host
        r(r"\b[a-z][a-z0-9+.-]*://[^\s:/@]+:([^\s@/]+)@", 1),
    ]
});

/// Returns the masked text and how many secrets were replaced.
pub fn mask(text: &str) -> (String, usize) {
    let mut out = text.to_string();
    let mut count = 0;
    for rule in RULES.iter() {
        let mut ranges: Vec<(usize, usize)> = rule
            .re
            .captures_iter(&out)
            .filter_map(|c| c.get(rule.group).map(|m| (m.start(), m.end())))
            .filter(|(s, e)| out[*s..*e] != *MASK)
            .collect();
        count += ranges.len();
        ranges.reverse();
        for (s, e) in ranges {
            out.replace_range(s..e, MASK);
        }
    }
    (out, count)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn masks_common_secrets() {
        let input = "APP_ENV=production\nDB_PASSWORD=pr0d-Sup3r\nAPI_TOKEN=sk-live-9f8a7b6c5d\nexport AWS_ACCESS_KEY_ID=AKIAABCDEFGHIJKLMNOP\ncurl -H 'Authorization: Bearer abcdefgh12345678' https://x\ngit clone https://bob:hunter22@github.com/x\n";
        let (out, n) = mask(input);
        assert!(out.contains("APP_ENV=production"), "non-secret kept");
        for secret in ["pr0d-Sup3r", "sk-live-9f8a7b6c5d", "AKIAABCDEFGHIJKLMNOP", "abcdefgh12345678", "hunter22"] {
            assert!(!out.contains(secret), "{secret} leaked:\n{out}");
        }
        assert_eq!(n, 5, "{out}");
    }

    #[test]
    fn masks_pem_blocks_and_provider_tokens() {
        let pem = "a\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXk\n-----END OPENSSH PRIVATE KEY-----\nz";
        let (out, n) = mask(pem);
        assert_eq!((out.as_str(), n), ("a\n••••••••\nz", 1));
        let (out, _) = mask("ghp_abcdefghijklmnopqrstuvwxyz0123 and xoxb-1234567890-abc");
        assert!(!out.contains("ghp_") && !out.contains("xoxb-"));
    }

    #[test]
    fn leaves_ordinary_output_alone() {
        let text = "total 48K\n-rw-r----- 1 www-data adm 28K access.log\nload average: 0.24\n토큰 없음";
        assert_eq!(mask(text), (text.to_string(), 0));
    }
}
