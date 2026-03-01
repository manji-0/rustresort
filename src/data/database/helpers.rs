use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use std::collections::HashSet;

pub(super) const OAUTH_ACCESS_TOKEN_HASH_PREFIX: &str = "sha256:";
const OAUTH_ACCESS_TOKEN_HASH_ENCODED_LEN: usize = 43;
const OAUTH_ACCESS_TOKEN_HASH_DECODED_LEN: usize = 32;

pub(super) fn poll_is_expired(expires_at: &str, persisted_expired: i64) -> bool {
    if persisted_expired != 0 {
        return true;
    }

    DateTime::parse_from_rfc3339(expires_at)
        .map(|parsed| parsed.with_timezone(&Utc) <= Utc::now())
        .unwrap_or(true)
}

fn is_hashtag_char(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_hashtag_boundary(previous: Option<char>) -> bool {
    previous
        .map(|c| !is_hashtag_char(c) && c != '&' && c != '/')
        .unwrap_or(true)
}

fn skip_html_tag(content: &str, start: usize) -> Option<usize> {
    if !content[start..].starts_with('<') {
        return None;
    }

    let mut quoted = None;
    for (offset, ch) in content[start + 1..].char_indices() {
        match (quoted, ch) {
            (Some(quote), _) if ch == quote => quoted = None,
            (Some(_), _) => {}
            (None, '"') | (None, '\'') => quoted = Some(ch),
            (None, '>') => return Some(start + 1 + offset + ch.len_utf8()),
            (None, _) => {}
        }
    }

    None
}

pub(super) fn extract_hashtags_from_content(content: &str) -> Vec<String> {
    let mut hashtags = Vec::new();
    let mut seen = HashSet::new();
    let mut chars = content.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        if ch != '#' {
            continue;
        }

        let previous = content[..index].chars().next_back();
        if !is_hashtag_boundary(previous) {
            continue;
        }

        let mut tag = String::new();
        let mut cursor = index + ch.len_utf8();
        loop {
            let Some(next_char) = content[cursor..].chars().next() else {
                break;
            };
            if is_hashtag_char(next_char) {
                tag.push(next_char.to_ascii_lowercase());
                cursor += next_char.len_utf8();
                continue;
            }
            if next_char == '<'
                && let Some(after_tag) = skip_html_tag(content, cursor)
            {
                cursor = after_tag;
                continue;
            }
            break;
        }

        while let Some((peek_index, _)) = chars.peek().copied() {
            if peek_index < cursor {
                chars.next();
            } else {
                break;
            }
        }

        if !tag.is_empty() && seen.insert(tag.clone()) {
            hashtags.push(tag);
        }
    }

    hashtags
}

fn parse_account_address(address: &str) -> Option<(String, String, Option<u16>)> {
    let (username, authority) = address.split_once('@')?;
    let parsed = url::Url::parse(&format!("http://{}", authority)).ok()?;
    let host = parsed.host_str()?;
    Some((
        username.to_ascii_lowercase(),
        host.to_ascii_lowercase(),
        extract_explicit_port(authority),
    ))
}

fn extract_explicit_port(authority: &str) -> Option<u16> {
    let authority = authority.trim();

    if let Some(rest) = authority.strip_prefix('[') {
        let (_, tail) = rest.split_once(']')?;
        let port_str = tail.strip_prefix(':')?;
        if port_str.is_empty() || !port_str.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        return port_str.parse::<u16>().ok();
    }

    let (host_part, port_str) = authority.rsplit_once(':')?;
    if host_part.is_empty()
        || host_part.contains(':')
        || port_str.is_empty()
        || !port_str.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }

    port_str.parse::<u16>().ok()
}

fn format_host_for_authority(host: &str) -> String {
    if host.contains(':') {
        format!("[{}]", host)
    } else {
        host.to_string()
    }
}

fn push_case_insensitive_unique(
    values: &mut Vec<String>,
    seen_casefold: &mut HashSet<String>,
    candidate: String,
) {
    if !seen_casefold.insert(candidate.to_ascii_lowercase()) {
        return;
    }
    values.push(candidate);
}

pub(super) fn equivalent_account_address_candidates(
    target_address: &str,
    default_port: Option<u16>,
) -> Vec<String> {
    let mut candidates = Vec::new();
    let mut seen_casefold = HashSet::new();
    push_case_insensitive_unique(
        &mut candidates,
        &mut seen_casefold,
        target_address.to_string(),
    );

    let Some((username, host, explicit_port)) = parse_account_address(target_address) else {
        return candidates;
    };
    let authority = format_host_for_authority(&host);
    let without_port = format!("{}@{}", username, authority);

    if let Some(port) = explicit_port {
        push_case_insensitive_unique(
            &mut candidates,
            &mut seen_casefold,
            format!("{}@{}:{}", username, authority, port),
        );

        if default_port == Some(port) {
            push_case_insensitive_unique(&mut candidates, &mut seen_casefold, without_port);
        }
    } else {
        push_case_insensitive_unique(&mut candidates, &mut seen_casefold, without_port);

        if let Some(default_port) = default_port {
            push_case_insensitive_unique(
                &mut candidates,
                &mut seen_casefold,
                format!("{}@{}:{}", username, authority, default_port),
            );
        }
    }

    candidates
}

pub(super) fn account_addresses_match(left: &str, right: &str, default_port: Option<u16>) -> bool {
    let Some((left_user, left_host, left_port)) = parse_account_address(left) else {
        return left.eq_ignore_ascii_case(right);
    };
    let Some((right_user, right_host, right_port)) = parse_account_address(right) else {
        return left.eq_ignore_ascii_case(right);
    };

    if left_user != right_user || left_host != right_host {
        return false;
    }

    match default_port {
        Some(port) => left_port.unwrap_or(port) == right_port.unwrap_or(port),
        None => left_port == right_port,
    }
}

pub(super) fn find_matching_addresses(
    candidates: &[String],
    target: &str,
    default_port: Option<u16>,
) -> Vec<String> {
    candidates
        .iter()
        .filter(|candidate| account_addresses_match(candidate, target, default_port))
        .cloned()
        .collect()
}

pub(super) fn parse_json_value(raw: Option<String>) -> Option<serde_json::Value> {
    raw.and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
}

pub(super) fn hash_oauth_access_token(access_token: &str) -> String {
    let digest = Sha256::digest(access_token.as_bytes());
    format!(
        "{}{}",
        OAUTH_ACCESS_TOKEN_HASH_PREFIX,
        URL_SAFE_NO_PAD.encode(digest)
    )
}

pub(super) fn is_hashed_oauth_access_token(stored_access_token: &str) -> bool {
    let Some(encoded_digest) = stored_access_token.strip_prefix(OAUTH_ACCESS_TOKEN_HASH_PREFIX)
    else {
        return false;
    };

    if encoded_digest.len() != OAUTH_ACCESS_TOKEN_HASH_ENCODED_LEN
        || !encoded_digest
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        return false;
    }

    URL_SAFE_NO_PAD
        .decode(encoded_digest)
        .map(|bytes| bytes.len() == OAUTH_ACCESS_TOKEN_HASH_DECODED_LEN)
        .unwrap_or(false)
}
