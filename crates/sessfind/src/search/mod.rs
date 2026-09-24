pub mod fts;
pub mod results;

pub(crate) fn terms_require_all(query: &str) -> bool {
    let tokens: Vec<&str> = query.split_whitespace().collect();
    if tokens.len() < 2 || tokens.contains(&"OR") {
        return false;
    }
    if tokens.contains(&"AND") {
        return true;
    }

    !query.contains(['"', '\'', '^', '~', '[', ']', '{', '}', '\\'])
        && !tokens.iter().any(|token| {
            token.starts_with(['+', '-']) || token.ends_with('*') || token.contains([':', '(', ')'])
        })
}

pub(crate) fn boolean_operator(token: &str) -> bool {
    matches!(token, "AND" | "OR" | "NOT")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_multiword_queries_require_all_terms() {
        assert!(terms_require_all("github pull request"));
        assert!(terms_require_all("github AND request"));
        assert!(!terms_require_all("github"));
        assert!(!terms_require_all("github OR request"));
        assert!(!terms_require_all("\"github pull request\""));
        assert!(!terms_require_all("+github request"));
        assert!(!terms_require_all("github request*"));
        assert!(!terms_require_all("github^2 request"));
        assert!(!terms_require_all("'github pull' request"));
    }
}
