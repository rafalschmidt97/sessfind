//! Credential redaction applied before session text reaches the search index
//! or a preview. Native session files are never modified.

use regex::{Captures, Regex};
use std::borrow::Cow;
use std::sync::LazyLock;

/// Bump when the rules change so existing indexes are rebuilt with them.
pub const REDACTION_VERSION: u32 = 1;

pub const PLACEHOLDER: &str = "[REDACTED]";

/// Credentials with a recognizable format, replaced whole.
static TOKEN_PATTERNS: LazyLock<Vec<Regex>> = LazyLock::new(|| {
    [
        // PEM/OpenSSH/PGP private key blocks
        r"-----BEGIN [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----[\s\S]*?-----END [A-Z0-9 ]*PRIVATE KEY(?: BLOCK)?-----",
        // AWS access key IDs
        r"\b(?:AKIA|ASIA|ABIA|ACCA)[0-9A-Z]{16}\b",
        // GitHub classic and fine-grained tokens
        r"\b(?:ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{36,255}\b",
        r"\bgithub_pat_[A-Za-z0-9_]{22,255}\b",
        // OpenAI and Anthropic API keys
        r"\bsk-[A-Za-z0-9_-]{20,}",
        // Slack tokens
        r"\bxox[abposr]-[A-Za-z0-9-]{10,}",
        // Stripe live keys
        r"\b(?:sk|rk)_live_[A-Za-z0-9]{16,}",
        // JWTs
        r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}",
    ]
    .iter()
    .map(|pattern| Regex::new(pattern).expect("valid redaction pattern"))
    .collect()
});

/// `Authorization: Bearer|Basic <value>` and bare `Bearer <value>`.
static AUTH_HEADER: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)(\bbearer\s+|\bauthorization:\s*basic\s+)[A-Za-z0-9._~+/=-]{16,}")
        .expect("valid redaction pattern")
});

/// Connection URLs keep scheme, user and host; only the password goes.
static URL_PASSWORD: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)\b((?:postgres(?:ql)?|mysql|mariadb|mongodb(?:\+srv)?|rediss?|amqps?|https?)://[^\s:/@]+:)[^\s@/]+@",
    )
    .expect("valid redaction pattern")
});

/// `NAME_TOKEN=value`, `"apiKey": "value"`, `X-API-KEY: value` and similar.
/// The key must end with a secret-like word, so `tokenizer = ...` or
/// `max_tokens: 4096` are left alone.
static ASSIGNMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?i)(\b[A-Za-z0-9_.-]*(?:password|passwd|passphrase|secret|token|api[_-]?key|access[_-]?key|private[_-]?key|account[_-]?key|sharedaccesskey)\b["']?\s*[:=]\s*["']?)([A-Za-z0-9_\-+/=.~!@#$%^&*]{8,})"#,
    )
    .expect("valid redaction pattern")
});

/// Replace credentials in `text` with [`PLACEHOLDER`]. Borrows when nothing matched.
pub fn redact(text: &str) -> Cow<'_, str> {
    let mut output = Cow::Borrowed(text);
    for pattern in TOKEN_PATTERNS.iter() {
        replace(&mut output, pattern, |_| PLACEHOLDER.to_string());
    }
    replace(&mut output, &AUTH_HEADER, |caps| {
        format!("{}{PLACEHOLDER}", &caps[1])
    });
    replace(&mut output, &URL_PASSWORD, |caps| {
        format!("{}{PLACEHOLDER}@", &caps[1])
    });
    replace(&mut output, &ASSIGNMENT, |caps| {
        let value = &caps[2];
        // Real secrets mix letters and digits; plain words such as
        // `password: required` or identifiers do not.
        let looks_secret = value.chars().any(|c| c.is_ascii_digit())
            && value.chars().any(|c| c.is_ascii_alphabetic());
        if looks_secret {
            format!("{}{PLACEHOLDER}", &caps[1])
        } else {
            caps[0].to_string()
        }
    });
    output
}

fn replace(text: &mut Cow<'_, str>, pattern: &Regex, f: impl Fn(&Captures) -> String) {
    if let Cow::Owned(replaced) = pattern.replace_all(text, |caps: &Captures| f(caps)) {
        *text = Cow::Owned(replaced);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_redacted(input: &str, secret: &str) {
        let output = redact(input);
        assert!(!output.contains(secret), "{secret} leaked in: {output}");
        assert!(output.contains(PLACEHOLDER), "no placeholder in: {output}");
    }

    #[test]
    fn redacts_known_token_formats() {
        let cases = [
            ("key AKIAIOSFODNN7EXAMPLE here", "AKIAIOSFODNN7EXAMPLE"),
            (
                "token ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789 used",
                "ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789",
            ),
            (
                "github_pat_11ABCDEFG0123456789_abcdefghijklmnopqrstuvwxyz",
                "github_pat_11ABCDEFG0123456789",
            ),
            (
                "ANTHROPIC: sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz",
                "sk-ant-api03-AbCdEfGhIjKlMnOpQrStUvWxYz",
            ),
            (
                "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz012345",
                "sk-proj-AbCdEfGhIjKlMnOpQrStUvWxYz012345",
            ),
            ("xoxb-1234567890-abcdefghij", "xoxb-1234567890-abcdefghij"),
            (
                "sk_live_AbCdEfGhIjKlMnOp1234",
                "sk_live_AbCdEfGhIjKlMnOp1234",
            ),
            (
                "jwt eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dozjgNryP4J3jVmNHl0w5N_XgL0n3I9PlFUP0THsR8U",
                "eyJzdWIiOiIxMjM0NTY3ODkwIn0",
            ),
        ];
        for (input, secret) in cases {
            assert_redacted(input, secret);
        }
    }

    #[test]
    fn redacts_private_key_block() {
        let input = "before\n-----BEGIN OPENSSH PRIVATE KEY-----\nb3BlbnNzaC1rZXktdjEAAAAA\nAAAAbm9uZQ\n-----END OPENSSH PRIVATE KEY-----\nafter";
        let output = redact(input);
        assert_eq!(output, format!("before\n{PLACEHOLDER}\nafter"));
    }

    #[test]
    fn keeps_auth_scheme_and_url_host() {
        assert_eq!(
            redact("curl -H 'Authorization: Bearer abcdef0123456789abcdef'"),
            format!("curl -H 'Authorization: Bearer {PLACEHOLDER}'")
        );
        assert_eq!(
            redact("Authorization: Basic dXNlcjpwYXNzd29yZDEyMw=="),
            format!("Authorization: Basic {PLACEHOLDER}")
        );
        assert_eq!(
            redact("postgres://admin:s3cr3t-pw@db.internal:5432/app"),
            format!("postgres://admin:{PLACEHOLDER}@db.internal:5432/app")
        );
    }

    #[test]
    fn redacts_secret_assignments() {
        assert_eq!(
            redact("export GRAFIQ_TOKEN=a1b2c3d4e5f6g7h8"),
            format!("export GRAFIQ_TOKEN={PLACEHOLDER}")
        );
        assert_eq!(
            redact(r#"{"apiKey": "Zx9Yw8Vu7Ts6"}"#),
            format!(r#"{{"apiKey": "{PLACEHOLDER}"}}"#)
        );
        assert_eq!(
            redact("X-API-KEY: 9f8e7d6c5b4a3210"),
            format!("X-API-KEY: {PLACEHOLDER}")
        );
        assert_eq!(
            redact("DB_PASSWORD='hunter2hunter2'"),
            format!("DB_PASSWORD='{PLACEHOLDER}'")
        );
    }

    #[test]
    fn leaves_ordinary_text_alone() {
        let inputs = [
            "let tokenizer = SimpleTokenizer::default();",
            "max_tokens: 4096",
            "password: required for login",
            "the secret = understanding the problem",
            "Bearer tokens are short-lived",
            "https://github.com/rafalschmidt97/sessfind/pull/12",
            "sk-learn is not a key",
        ];
        for input in inputs {
            assert_eq!(redact(input), input);
        }
    }

    #[test]
    fn redaction_is_idempotent() {
        let once =
            redact("GRAFIQ_TOKEN=a1b2c3d4e5f6g7h8 and ghp_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789")
                .into_owned();
        assert_eq!(redact(&once), once);
    }
}
