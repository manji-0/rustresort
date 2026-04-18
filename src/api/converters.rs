//! Conversion functions from database models to API DTOs

use crate::api::dto::*;
use crate::config::AppConfig;
use crate::data::{Account, Database, MediaAttachment, RemoteStatusAttachment, Status};
use crate::error::AppError;
use chrono::Utc;
use std::collections::HashMap;

/// Local account counters used in API responses.
#[derive(Debug, Clone, Copy, Default)]
pub struct AccountStats {
    pub followers_count: i32,
    pub following_count: i32,
    pub statuses_count: i32,
}

/// Remote account counters used for status response placeholders.
#[derive(Debug, Clone, Default)]
pub struct RemoteAccountStats {
    pub followers_count: i32,
    pub following_count: i32,
    pub statuses_count: i32,
    pub force_sensitive: bool,
    pub uri: Option<String>,
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub profile_fields_json: Option<String>,
    pub avatar_url: Option<String>,
    pub header_url: Option<String>,
    pub locked: bool,
    pub bot: bool,
    pub discoverable: bool,
    pub indexable: bool,
}

fn saturating_i32(value: i64) -> i32 {
    if value > i64::from(i32::MAX) {
        i32::MAX
    } else if value < i64::from(i32::MIN) {
        i32::MIN
    } else {
        value as i32
    }
}

fn saturating_u64_i32(value: u64) -> i32 {
    if value > i32::MAX as u64 {
        i32::MAX
    } else {
        value as i32
    }
}

fn default_port_for_protocol(protocol: &str) -> Option<u16> {
    match protocol {
        "http" => Some(80),
        "https" => Some(443),
        _ => None,
    }
}

fn status_content_to_source_text(content: &str) -> String {
    let normalized = content
        .replace("<br />", "\n")
        .replace("<br/>", "\n")
        .replace("<br>", "\n")
        .replace("</p>", "\n\n")
        .replace("<p>", "");

    let mut without_tags = String::with_capacity(normalized.len());
    let mut in_tag = false;
    for ch in normalized.chars() {
        match ch {
            '<' => in_tag = true,
            '>' if in_tag => in_tag = false,
            _ if !in_tag => without_tags.push(ch),
            _ => {}
        }
    }

    let decoded = html_escape::decode_html_entities(without_tags.trim()).into_owned();
    let mut lines = decoded.lines().map(str::trim_end).peekable();
    let mut output = String::new();
    let mut previous_blank = false;
    while let Some(line) = lines.next() {
        let is_blank = line.trim().is_empty();
        if is_blank {
            if !previous_blank && lines.peek().is_some() {
                output.push('\n');
            }
        } else {
            if !output.is_empty() {
                output.push('\n');
            }
            output.push_str(line.trim_end());
        }
        previous_blank = is_blank;
    }
    output
}

fn first_url_from_text(text: &str) -> Option<String> {
    text.split_whitespace().find_map(|token| {
        let trimmed = token
            .trim_matches(|ch: char| {
                matches!(
                    ch,
                    '<' | '>' | '"' | '\'' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | '.'
                )
            })
            .trim();
        (!trimmed.is_empty()
            && (trimmed.starts_with("https://") || trimmed.starts_with("http://"))
            && url::Url::parse(trimmed).is_ok())
        .then(|| trimmed.to_string())
    })
}

pub fn build_status_card_value(status: &Status) -> Option<serde_json::Value> {
    let text = status_content_to_source_text(&status.content);
    let url = first_url_from_text(&text)?;
    let parsed = url::Url::parse(&url).ok()?;
    let provider_name = parsed.host_str().unwrap_or_default().to_string();
    Some(serde_json::json!({
        "url": url,
        "title": provider_name,
        "description": "",
        "type": "link",
        "author_name": "",
        "author_url": "",
        "provider_name": provider_name,
        "provider_url": format!("{}://{}", parsed.scheme(), parsed.host_str().unwrap_or_default()),
        "html": "",
        "width": 0,
        "height": 0,
        "image": serde_json::Value::Null,
        "embed_url": "",
        "blurhash": serde_json::Value::Null,
    }))
}

async fn resolve_in_reply_to_account_id(
    db: &Database,
    status: &Status,
    local_account_id: &str,
) -> Result<Option<String>, AppError> {
    let current_author = if status.is_local || status.account_address.trim().is_empty() {
        "__local__".to_string()
    } else {
        status.account_address.trim().to_ascii_lowercase()
    };
    let mut next_parent_uri = status.in_reply_to_uri.clone();

    while let Some(parent_uri) = next_parent_uri {
        let Some(parent_status) = db.get_status_by_uri(&parent_uri).await? else {
            return Ok(None);
        };
        if parent_status.is_local || parent_status.account_address.trim().is_empty() {
            return Ok(Some(local_account_id.to_string()));
        }
        let parent_author =
            if parent_status.is_local || parent_status.account_address.trim().is_empty() {
                "__local__".to_string()
            } else {
                parent_status.account_address.trim().to_ascii_lowercase()
            };
        if parent_author != current_author {
            return Ok(Some(parent_status.account_address));
        }
        next_parent_uri = parent_status.in_reply_to_uri.clone();
    }

    Ok(None)
}

fn build_status_tags(content: &str, base_url: &str) -> Vec<serde_json::Value> {
    crate::data::extract_hashtags_from_content(content)
        .into_iter()
        .map(|name| {
            serde_json::json!({
                "name": name,
                "url": format!("{}/tags/{}", base_url, name),
            })
        })
        .collect()
}

fn is_mention_boundary(previous: Option<char>) -> bool {
    previous
        .map(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '@'))
        .unwrap_or(true)
}

fn extract_mentions_from_text(text: &str) -> Vec<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut mentions = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut index = 0;

    while index < chars.len() {
        if chars[index] != '@' || !is_mention_boundary(index.checked_sub(1).map(|i| chars[i])) {
            index += 1;
            continue;
        }

        let mut cursor = index + 1;
        let mut username = String::new();
        while cursor < chars.len() {
            let ch = chars[cursor];
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '.' || ch == '-' {
                username.push(ch);
                cursor += 1;
            } else {
                break;
            }
        }
        if username.is_empty() {
            index += 1;
            continue;
        }

        let mut account = username;
        if cursor < chars.len() && chars[cursor] == '@' {
            cursor += 1;
            let mut domain = String::new();
            while cursor < chars.len() {
                let ch = chars[cursor];
                if ch.is_ascii_alphanumeric() || ch == '.' || ch == '-' {
                    domain.push(ch);
                    cursor += 1;
                } else {
                    break;
                }
            }
            if !domain.is_empty() {
                account.push('@');
                account.push_str(&domain);
            }
        }

        let normalized = account.to_ascii_lowercase();
        if seen.insert(normalized.clone()) {
            mentions.push(normalized);
        }
        index = cursor;
    }

    mentions
}

fn build_status_mentions(
    content: &str,
    base_url: &str,
    local_username: &str,
    local_account_id: &str,
) -> Vec<serde_json::Value> {
    let text = status_content_to_source_text(content);
    extract_mentions_from_text(&text)
        .into_iter()
        .map(|acct| {
            let (username, url) = if let Some((username, domain)) = acct.split_once('@') {
                (
                    username.to_string(),
                    format!("https://{}/@{}", domain.to_ascii_lowercase(), username),
                )
            } else {
                (
                    acct.clone(),
                    format!("{}/users/{}", base_url, acct.to_ascii_lowercase()),
                )
            };
            let id = if acct == local_username {
                local_account_id.to_string()
            } else if acct.contains('@') {
                acct.clone()
            } else {
                format!("{}/users/{}", base_url, acct)
            };
            serde_json::json!({
                "id": id,
                "username": username,
                "url": url,
                "acct": acct,
            })
        })
        .collect()
}

async fn enrich_status_response(
    db: &Database,
    status: &Status,
    response: &mut StatusResponse,
) -> Result<(), AppError> {
    response.replies_count = saturating_i32(db.count_replies_by_uri(&status.uri).await?);
    response.reblogs_count = saturating_i32(db.count_reposts(&status.id).await?);
    response.favourites_count = saturating_i32(db.count_favourites(&status.id).await?);
    response.quotes_count = saturating_i32(db.count_quotes_by_uri(&status.uri).await?);
    response.edited_at = db.get_latest_status_edit_at(&status.id).await?;
    Ok(())
}

async fn load_remote_account_stats_for_status(
    db: &Database,
    local_protocol: &str,
    status: &Status,
) -> Result<Option<RemoteAccountStats>, AppError> {
    let account_address = status.account_address.trim();
    if status.is_local || account_address.is_empty() {
        return Ok(None);
    }

    let statuses_count = db
        .count_statuses_by_account_address_with_default_port(
            account_address,
            default_port_for_protocol(local_protocol),
        )
        .await?;

    Ok(Some(RemoteAccountStats {
        statuses_count: saturating_i32(statuses_count),
        force_sensitive: db
            .is_account_sensitive(account_address, default_port_for_protocol(local_protocol))
            .await?,
        ..RemoteAccountStats::default()
    }))
}

async fn build_quote_response_value(
    db: &Database,
    quote_status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
) -> Result<serde_json::Value, AppError> {
    let remote_account_stats =
        load_remote_account_stats_for_status(db, &config.server.protocol, quote_status).await?;
    let media_attachments = db.get_media_by_status(&quote_status.id).await?;
    let media_attachment_responses =
        load_status_media_attachment_responses(db, &quote_status.id, config, &media_attachments)
            .await?;
    let force_sensitive = remote_account_stats
        .as_ref()
        .map(|stats| stats.force_sensitive)
        .unwrap_or(false);
    let mut response = status_to_response_with_media(
        quote_status,
        account,
        config,
        account_stats,
        remote_account_stats,
        StatusInteractions::default(),
        force_sensitive,
        &media_attachment_responses,
    );
    enrich_status_response(db, quote_status, &mut response).await?;
    response.quote = None;
    response.quote_approval = None;
    serde_json::to_value(response)
        .map_err(|error| AppError::serialization("quoted status response serialization", error))
}

async fn build_reblogged_status_response(
    db: &Database,
    boosted_status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
) -> Result<StatusResponse, AppError> {
    let remote_account_stats =
        load_remote_account_stats_for_status(db, &config.server.protocol, boosted_status).await?;
    let media_attachments = db.get_media_by_status(&boosted_status.id).await?;
    let media_attachment_responses =
        load_status_media_attachment_responses(db, &boosted_status.id, config, &media_attachments)
            .await?;
    let force_sensitive = remote_account_stats
        .as_ref()
        .map(|stats| stats.force_sensitive)
        .unwrap_or(false);
    let thread_uri = db.resolve_thread_root_uri(boosted_status).await?;
    let mut response = status_to_response_with_media(
        boosted_status,
        account,
        config,
        account_stats,
        remote_account_stats,
        StatusInteractions::new(
            Some(db.is_favourited(&boosted_status.id).await?),
            Some(db.is_reposted(&boosted_status.id).await?),
            Some(db.is_thread_muted(&thread_uri).await?),
            Some(db.is_bookmarked(&boosted_status.id).await?),
            Some(db.is_status_pinned(&boosted_status.id).await?),
        ),
        force_sensitive,
        &media_attachment_responses,
    );
    enrich_status_response(db, boosted_status, &mut response).await?;
    response.in_reply_to_account_id =
        resolve_in_reply_to_account_id(db, boosted_status, &account.id).await?;
    response.poll = load_status_poll_response(db, &boosted_status.id, account, config).await?;
    response.filtered = load_status_filtered(db, boosted_status).await?;
    response.card = build_status_card_value(boosted_status);
    if let Some(quote_of_uri) = boosted_status.quote_of_uri.as_deref()
        && let Some(quote_status) = db.get_status_by_uri(quote_of_uri).await?
    {
        response.quote = Some(
            build_quote_response_value(db, &quote_status, account, config, account_stats).await?,
        );
    }
    response.reblog = None;
    Ok(response)
}

pub async fn build_status_response_with_media(
    db: &Database,
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
    media_attachments: &[MediaAttachment],
) -> Result<StatusResponse, AppError> {
    let media_attachment_responses =
        load_status_media_attachment_responses(db, &status.id, config, media_attachments).await?;
    let force_sensitive = remote_account_stats
        .as_ref()
        .map(|stats| stats.force_sensitive)
        .unwrap_or(false);
    let mut response = status_to_response_with_media(
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
        force_sensitive,
        &media_attachment_responses,
    );
    enrich_status_response(db, status, &mut response).await?;
    response.in_reply_to_account_id =
        resolve_in_reply_to_account_id(db, status, &account.id).await?;
    if let Some(boost_of_uri) = status.boost_of_uri.as_deref()
        && boost_of_uri != status.uri
        && let Some(boosted_status) = db.get_status_by_uri(boost_of_uri).await?
    {
        let boosted_response =
            build_reblogged_status_response(db, &boosted_status, account, config, account_stats)
                .await?;
        response.reblog = Some(Box::new(boosted_response));
    }
    response.poll = load_status_poll_response(db, &status.id, account, config).await?;
    response.filtered = load_status_filtered(db, status).await?;
    response.card = build_status_card_value(status);
    if let Some(quote_of_uri) = status.quote_of_uri.as_deref()
        && let Some(quote_status) = db.get_status_by_uri(quote_of_uri).await?
    {
        response.quote = Some(
            build_quote_response_value(db, &quote_status, account, config, account_stats).await?,
        );
    }
    Ok(response)
}

fn phrase_matches_text(text: &str, phrase: &str, whole_word: bool) -> bool {
    if phrase.is_empty() {
        return false;
    }
    if !whole_word {
        return text.contains(phrase);
    }

    let mut start = 0usize;
    while let Some(relative_idx) = text[start..].find(phrase) {
        let idx = start + relative_idx;
        let before = text[..idx].chars().next_back();
        let after = text[idx + phrase.len()..].chars().next();
        let before_ok = before.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_');
        let after_ok = after.is_none_or(|ch| !ch.is_alphanumeric() && ch != '_');
        if before_ok && after_ok {
            return true;
        }
        start = idx + phrase.len();
    }
    false
}

async fn load_status_filtered(
    db: &Database,
    status: &Status,
) -> Result<Vec<serde_json::Value>, AppError> {
    let filters = db.get_all_filters().await?;
    if filters.is_empty() {
        return Ok(Vec::new());
    }

    let text = format!(
        "{}\n{}",
        status_content_to_source_text(&status.content),
        status.content_warning.clone().unwrap_or_default()
    )
    .to_ascii_lowercase();
    let mut filtered = Vec::new();

    for (id, phrase, context, expires_at, irreversible, whole_word) in filters {
        if expires_at.as_deref().is_some_and(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .map(|datetime| datetime.with_timezone(&Utc) <= Utc::now())
                .unwrap_or(false)
        }) {
            continue;
        }

        let keywords = db.get_filter_keywords(&id).await?;
        let status_filters = db.get_filter_statuses(&id).await?;
        let mut keyword_matches = Vec::new();

        if keywords.is_empty() {
            let normalized_phrase = phrase.trim().to_ascii_lowercase();
            if phrase_matches_text(&text, &normalized_phrase, whole_word) {
                keyword_matches.push(phrase.clone());
            }
        } else {
            for (_keyword_id, keyword, keyword_whole_word) in keywords {
                let normalized_keyword = keyword.trim().to_ascii_lowercase();
                if phrase_matches_text(&text, &normalized_keyword, keyword_whole_word) {
                    keyword_matches.push(keyword);
                }
            }
        }

        let status_matches = status_filters
            .into_iter()
            .filter_map(|(_filter_status_id, status_id)| {
                (status_id == status.id).then_some(status_id)
            })
            .collect::<Vec<_>>();
        if keyword_matches.is_empty() && status_matches.is_empty() {
            continue;
        }

        let contexts = context
            .split(',')
            .map(|value| value.trim())
            .filter(|value| !value.is_empty())
            .map(|value| value.to_string())
            .collect::<Vec<_>>();
        filtered.push(serde_json::json!({
            "filter": {
                "id": id,
                "title": phrase,
                "context": contexts,
                "expires_at": expires_at,
                "filter_action": if irreversible { "hide" } else { "warn" },
            },
            "keyword_matches": if keyword_matches.is_empty() { serde_json::Value::Null } else { serde_json::json!(keyword_matches) },
            "status_matches": if status_matches.is_empty() { serde_json::Value::Null } else { serde_json::json!(status_matches) },
        }));
    }

    Ok(filtered)
}

pub async fn build_status_response_with_account_stats_and_remote_stats(
    db: &Database,
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
) -> Result<StatusResponse, AppError> {
    build_status_response_with_media(
        db,
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
        &[],
    )
    .await
}

pub async fn build_status_response_with_account_stats(
    db: &Database,
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    interactions: StatusInteractions,
) -> Result<StatusResponse, AppError> {
    build_status_response_with_account_stats_and_remote_stats(
        db,
        status,
        account,
        config,
        account_stats,
        None,
        interactions,
    )
    .await
}

/// Load local account counters from the database.
pub async fn load_local_account_stats(db: &Database) -> Result<AccountStats, AppError> {
    Ok(AccountStats {
        followers_count: saturating_i32(db.count_follower_addresses().await?),
        following_count: saturating_i32(db.count_follow_addresses().await?),
        statuses_count: saturating_i32(db.count_local_statuses().await?),
    })
}

/// Load remote account status counters for a status list.
///
/// Returns a map keyed by the raw status account address.
pub async fn load_remote_statuses_count_map(
    db: &Database,
    local_protocol: &str,
    statuses: &[Status],
) -> Result<HashMap<String, i32>, AppError> {
    let mut counts = HashMap::new();
    let default_port = default_port_for_protocol(local_protocol);

    for status in statuses {
        if status.is_local {
            continue;
        }
        let account_address = status.account_address.trim();
        if account_address.is_empty() || counts.contains_key(account_address) {
            continue;
        }

        let count = db
            .count_statuses_by_account_address_with_default_port(account_address, default_port)
            .await?;
        counts.insert(account_address.to_string(), saturating_i32(count));
    }

    Ok(counts)
}

/// Load remote account counters for a status list.
///
/// Returns a map keyed by the raw status account address.
pub async fn load_remote_account_stats_map(
    db: &Database,
    profile_cache: &crate::data::ProfileCache,
    local_protocol: &str,
    statuses: &[Status],
) -> Result<HashMap<String, RemoteAccountStats>, AppError> {
    let mut account_stats_map = HashMap::new();
    let default_port = default_port_for_protocol(local_protocol);

    for status in statuses {
        if status.is_local {
            continue;
        }
        let account_address = status.account_address.trim();
        if account_address.is_empty() || account_stats_map.contains_key(account_address) {
            continue;
        }

        let statuses_count = db
            .count_statuses_by_account_address_with_default_port(account_address, default_port)
            .await?;
        let mut remote_stats = RemoteAccountStats {
            statuses_count: saturating_i32(statuses_count),
            force_sensitive: db
                .is_account_sensitive(account_address, default_port)
                .await?,
            ..RemoteAccountStats::default()
        };

        let mut profile = profile_cache.get(account_address).await;
        if profile.is_none() {
            let normalized_address = account_address.to_ascii_lowercase();
            if normalized_address != account_address {
                profile = profile_cache.get(&normalized_address).await;
            }
        }

        if let Some(profile) = profile {
            remote_stats.followers_count =
                profile.followers_count.map(saturating_u64_i32).unwrap_or(0);
            remote_stats.following_count =
                profile.following_count.map(saturating_u64_i32).unwrap_or(0);
            remote_stats.uri = Some(profile.uri.clone());
            remote_stats.display_name = profile.display_name.clone();
            remote_stats.note = profile.note.clone();
            remote_stats.profile_fields_json = profile.profile_fields_json.clone();
            remote_stats.avatar_url = profile.avatar_url.clone();
            remote_stats.header_url = profile.header_url.clone();
            remote_stats.locked = profile.locked;
            remote_stats.bot = profile.bot;
            remote_stats.discoverable = profile.discoverable;
            remote_stats.indexable = profile.indexable;
        }

        account_stats_map.insert(account_address.to_string(), remote_stats);
    }

    Ok(account_stats_map)
}

/// Convert Account to AccountResponse
#[cfg(test)]
pub fn account_to_response(account: &Account, config: &AppConfig) -> AccountResponse {
    account_to_response_with_stats(account, config, AccountStats::default())
}

/// Convert Account to AccountResponse with explicit counters.
pub fn account_to_response_with_stats(
    account: &Account,
    config: &AppConfig,
    stats: AccountStats,
) -> AccountResponse {
    let base_url = config.server.base_url();
    let media_url = &config.storage.media.public_url;

    AccountResponse {
        id: account.id.to_string(),
        username: account.username.clone(),
        acct: account.username.clone(), // Local account, no @domain
        uri: format!("{}/users/{}", base_url, account.username),
        display_name: account
            .display_name
            .clone()
            .unwrap_or_else(|| account.username.clone()),
        locked: account.locked,
        bot: account.bot,
        discoverable: account.discoverable,
        group: false,
        indexable: account.indexable,
        created_at: account.created_at,
        note: account.note.clone().unwrap_or_default(),
        url: format!("{}/users/{}", base_url, account.username),
        avatar: account
            .avatar_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        avatar_static: account
            .avatar_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        header: account
            .header_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        header_static: account
            .header_s3_key
            .as_ref()
            .map(|key| format!("{}/{}", media_url, key))
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        followers_count: stats.followers_count,
        following_count: stats.following_count,
        statuses_count: stats.statuses_count,
        last_status_at: None,
        emojis: vec![],
        fields: crate::profile_fields::profile_fields_for_response(
            account.profile_fields_json.as_deref(),
        ),
        roles: vec![],
        moved: None,
        source: None,
    }
}

fn remote_account_to_response(
    status: &Status,
    config: &AppConfig,
    remote_stats: Option<&RemoteAccountStats>,
) -> AccountResponse {
    let placeholder_created_at = chrono::DateTime::from_timestamp(0, 0)
        .expect("unix epoch timestamp should always be valid");
    let media_url = &config.storage.media.public_url;
    let address = status.account_address.trim();
    let (username, domain) = address
        .split_once('@')
        .unwrap_or(("unknown", "unknown.invalid"));
    let normalized_username = username.to_ascii_lowercase();
    let normalized_domain = domain.to_ascii_lowercase();
    let acct = format!("{}@{}", normalized_username, normalized_domain);

    let stats = remote_stats.cloned().unwrap_or_default();

    AccountResponse {
        id: acct.clone(),
        username: normalized_username.clone(),
        acct,
        uri: stats.uri.unwrap_or_else(|| {
            format!(
                "https://{}/users/{}",
                normalized_domain, normalized_username
            )
        }),
        display_name: stats
            .display_name
            .unwrap_or_else(|| normalized_username.clone()),
        locked: stats.locked,
        bot: stats.bot,
        discoverable: stats.discoverable,
        group: false,
        indexable: stats.indexable,
        // Remote account creation timestamp is unavailable; use a deterministic placeholder.
        created_at: placeholder_created_at,
        note: stats.note.unwrap_or_default(),
        url: format!("https://{}/@{}", normalized_domain, normalized_username),
        avatar: stats
            .avatar_url
            .clone()
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        avatar_static: stats
            .avatar_url
            .unwrap_or_else(|| format!("{}/default-avatar.png", media_url)),
        header: stats
            .header_url
            .clone()
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        header_static: stats
            .header_url
            .unwrap_or_else(|| format!("{}/default-header.png", media_url)),
        followers_count: stats.followers_count,
        following_count: stats.following_count,
        statuses_count: stats.statuses_count,
        last_status_at: None,
        emojis: vec![],
        fields: crate::profile_fields::profile_fields_for_response(
            stats.profile_fields_json.as_deref(),
        ),
        roles: vec![],
        moved: None,
        source: None,
    }
}

fn local_status_id_from_uri(uri: &str, config: &AppConfig) -> Option<String> {
    let parsed = url::Url::parse(uri).ok()?;
    let local_base = url::Url::parse(&config.server.base_url()).ok()?;

    let parsed_host = parsed
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    let local_host = local_base
        .host_str()?
        .trim_end_matches('.')
        .to_ascii_lowercase();
    if !parsed.scheme().eq_ignore_ascii_case(local_base.scheme())
        || parsed_host != local_host
        || parsed.port_or_known_default() != local_base.port_or_known_default()
    {
        return None;
    }

    let segments = parsed.path_segments()?.collect::<Vec<_>>();
    if segments.len() == 4 && segments[0] == "users" && segments[2] == "statuses" {
        let status_id = segments[3];
        if !status_id.is_empty() {
            return Some(status_id.to_string());
        }
    }
    None
}

fn media_type_from_content_type(content_type: &str) -> &'static str {
    if content_type.starts_with("image/") {
        "image"
    } else if content_type.starts_with("video/") {
        "video"
    } else if content_type.starts_with("audio/") {
        "audio"
    } else {
        "unknown"
    }
}

fn media_url(media_base_url: &str, s3_key: &str) -> String {
    format!(
        "{}/{}",
        media_base_url.trim_end_matches('/'),
        s3_key.trim_start_matches('/')
    )
}

fn media_attachment_to_response(
    attachment: &MediaAttachment,
    config: &AppConfig,
) -> MediaAttachmentResponse {
    let media_base_url = &config.storage.media.public_url;
    let url = media_url(media_base_url, &attachment.s3_key);
    let preview_url = attachment
        .thumbnail_s3_key
        .as_deref()
        .map(|key| media_url(media_base_url, key))
        .unwrap_or_else(|| url.clone());

    let meta = if attachment.width.is_some()
        || attachment.height.is_some()
        || attachment.focus_x.is_some()
        || attachment.focus_y.is_some()
    {
        Some(serde_json::json!({
            "original": {
                "width": attachment.width,
                "height": attachment.height,
                "size": attachment.width.zip(attachment.height).map(|(w, h)| format!("{w}x{h}")),
                "aspect": attachment.width.zip(attachment.height).and_then(|(w, h)| (h != 0).then_some(w as f64 / h as f64)),
                "focus": attachment.focus_x.zip(attachment.focus_y).map(|(x, y)| format!("{x:.3},{y:.3}")),
            }
        }))
    } else {
        None
    };

    MediaAttachmentResponse {
        id: attachment.id.clone(),
        media_type: media_type_from_content_type(&attachment.content_type).to_string(),
        url,
        preview_url,
        remote_url: None,
        text_url: None,
        meta,
        description: attachment.description.clone(),
        blurhash: attachment.blurhash.clone(),
    }
}

fn remote_media_attachment_to_response(
    attachment: &RemoteStatusAttachment,
) -> MediaAttachmentResponse {
    let url = attachment.remote_url.clone();
    let preview_url = attachment
        .preview_url
        .clone()
        .unwrap_or_else(|| url.clone());

    let meta = if attachment.width.is_some() || attachment.height.is_some() {
        Some(serde_json::json!({
            "original": {
                "width": attachment.width,
                "height": attachment.height,
                "size": attachment.width.zip(attachment.height).map(|(w, h)| format!("{w}x{h}")),
                "aspect": attachment.width.zip(attachment.height).and_then(|(w, h)| (h != 0).then_some(w as f64 / h as f64)),
            }
        }))
    } else {
        None
    };

    MediaAttachmentResponse {
        id: attachment.id.clone(),
        media_type: media_type_from_content_type(&attachment.content_type).to_string(),
        url: url.clone(),
        preview_url,
        remote_url: Some(url),
        text_url: None,
        meta,
        description: attachment.description.clone(),
        blurhash: attachment.blurhash.clone(),
    }
}

async fn load_status_media_attachment_responses(
    db: &Database,
    status_id: &str,
    config: &AppConfig,
    local_media_attachments: &[MediaAttachment],
) -> Result<Vec<MediaAttachmentResponse>, AppError> {
    let mut responses = local_media_attachments
        .iter()
        .map(|attachment| media_attachment_to_response(attachment, config))
        .collect::<Vec<_>>();

    responses.extend(
        db.get_remote_status_attachments(status_id)
            .await?
            .into_iter()
            .map(|attachment| remote_media_attachment_to_response(&attachment)),
    );

    Ok(responses)
}

async fn load_status_poll_response(
    db: &Database,
    status_id: &str,
    account: &Account,
    config: &AppConfig,
) -> Result<Option<serde_json::Value>, AppError> {
    let Some((poll_id, expires_at, expired, multiple, hide_totals, votes_count, voters_count)) =
        db.get_poll_by_status_id(status_id).await?
    else {
        return Ok(None);
    };

    let options = db.get_poll_options(&poll_id).await?;
    let account_address = format!("{}@{}", account.username, config.server.domain);
    let user_votes = db.get_user_poll_votes(&poll_id, &account_address).await?;
    let own_votes = user_votes
        .iter()
        .filter_map(|vote_option_id| {
            options
                .iter()
                .position(|(option_id, _, _)| option_id == vote_option_id)
        })
        .collect::<Vec<_>>();
    let options_response = options
        .into_iter()
        .map(|(_, title, option_votes_count)| {
            serde_json::json!({
                "title": title,
                "votes_count": option_votes_count,
            })
        })
        .collect::<Vec<_>>();

    Ok(Some(serde_json::json!({
        "id": poll_id,
        "expires_at": expires_at,
        "expired": expired,
        "multiple": multiple,
        "hide_totals": hide_totals,
        "votes_count": votes_count,
        "voters_count": voters_count,
        "voted": !own_votes.is_empty(),
        "own_votes": own_votes,
        "options": options_response,
        "emojis": [],
    })))
}

fn boost_stub_status(
    boost_of_uri: &str,
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
) -> StatusResponse {
    let placeholder_created_at = chrono::DateTime::from_timestamp(0, 0)
        .expect("unix epoch timestamp should always be valid");
    let media_url = &config.storage.media.public_url;
    let remote_stats = remote_account_stats.as_ref();
    let boost_account = if local_status_id_from_uri(boost_of_uri, config).is_some() {
        account_to_response_with_stats(account, config, account_stats)
    } else if let Ok(parsed) = url::Url::parse(boost_of_uri) {
        let normalized_host = parsed
            .host_str()
            .map(|host| host.trim_start_matches('[').trim_end_matches(']'))
            .map(str::to_ascii_lowercase)
            .filter(|host| !host.is_empty())
            .unwrap_or_else(|| "unknown.invalid".to_string());
        let authority_host = if normalized_host.contains(':') {
            format!("[{}]", normalized_host)
        } else {
            normalized_host.clone()
        };
        let authority = match parsed.port() {
            Some(port) => format!("{}:{port}", authority_host),
            None => authority_host,
        };
        let normalized_username = parsed
            .path_segments()
            .and_then(|segments| {
                let collected = segments.collect::<Vec<_>>();
                collected
                    .windows(2)
                    .find_map(|window| (window[0] == "users").then_some(window[1]))
                    .or_else(|| {
                        collected
                            .iter()
                            .find_map(|segment| segment.strip_prefix('@'))
                    })
            })
            .map(str::to_ascii_lowercase)
            .filter(|username| !username.is_empty())
            .unwrap_or_else(|| "unknown".to_string());
        let acct = format!("{}@{}", normalized_username, authority);
        let profile_url = format!(
            "{}://{}/@{}",
            parsed.scheme(),
            authority,
            normalized_username
        );
        AccountResponse {
            id: acct.clone(),
            username: normalized_username.clone(),
            acct,
            uri: format!(
                "{}://{}/users/{}",
                parsed.scheme(),
                authority,
                normalized_username
            ),
            display_name: normalized_username,
            locked: false,
            bot: false,
            discoverable: true,
            group: false,
            indexable: true,
            created_at: placeholder_created_at,
            note: String::new(),
            url: profile_url,
            avatar: format!("{}/default-avatar.png", media_url),
            avatar_static: format!("{}/default-avatar.png", media_url),
            header: format!("{}/default-header.png", media_url),
            header_static: format!("{}/default-header.png", media_url),
            followers_count: remote_stats.map(|stats| stats.followers_count).unwrap_or(0),
            following_count: remote_stats.map(|stats| stats.following_count).unwrap_or(0),
            statuses_count: remote_stats.map(|stats| stats.statuses_count).unwrap_or(0),
            last_status_at: None,
            emojis: vec![],
            fields: vec![],
            roles: vec![],
            moved: None,
            source: None,
        }
    } else {
        AccountResponse {
            id: "unknown@unknown.invalid".to_string(),
            username: "unknown".to_string(),
            acct: "unknown@unknown.invalid".to_string(),
            uri: boost_of_uri.to_string(),
            display_name: "unknown".to_string(),
            locked: false,
            bot: false,
            discoverable: true,
            group: false,
            indexable: true,
            created_at: placeholder_created_at,
            note: String::new(),
            url: boost_of_uri.to_string(),
            avatar: format!("{}/default-avatar.png", media_url),
            avatar_static: format!("{}/default-avatar.png", media_url),
            header: format!("{}/default-header.png", media_url),
            header_static: format!("{}/default-header.png", media_url),
            followers_count: remote_stats.map(|stats| stats.followers_count).unwrap_or(0),
            following_count: remote_stats.map(|stats| stats.following_count).unwrap_or(0),
            statuses_count: remote_stats.map(|stats| stats.statuses_count).unwrap_or(0),
            last_status_at: None,
            emojis: vec![],
            fields: vec![],
            roles: vec![],
            moved: None,
            source: None,
        }
    };

    StatusResponse {
        id: local_status_id_from_uri(boost_of_uri, config)
            .unwrap_or_else(|| boost_of_uri.to_string()),
        created_at: placeholder_created_at,
        in_reply_to_id: None,
        in_reply_to_account_id: None,
        sensitive: false,
        spoiler_text: String::new(),
        visibility: status.visibility.to_string(),
        language: status.language.clone(),
        uri: boost_of_uri.to_string(),
        url: boost_of_uri.to_string(),
        replies_count: 0,
        reblogs_count: 0,
        favourites_count: 0,
        quotes_count: 0,
        edited_at: None,
        content: String::new(),
        text: String::new(),
        reblog: None,
        application: None,
        account: boost_account,
        media_attachments: vec![],
        mentions: vec![],
        tags: vec![],
        emojis: vec![],
        quote: None,
        quote_approval: None,
        card: None,
        poll: None,
        filtered: vec![],
        favourited: false,
        reblogged: false,
        muted: false,
        bookmarked: false,
        pinned: false,
    }
}

/// Convert Status to StatusResponse
#[derive(Debug, Clone, Copy, Default)]
pub struct StatusInteractions {
    pub favourited: Option<bool>,
    pub reblogged: Option<bool>,
    pub muted: Option<bool>,
    pub bookmarked: Option<bool>,
    pub pinned: Option<bool>,
}

impl StatusInteractions {
    pub const fn new(
        favourited: Option<bool>,
        reblogged: Option<bool>,
        muted: Option<bool>,
        bookmarked: Option<bool>,
        pinned: Option<bool>,
    ) -> Self {
        Self {
            favourited,
            reblogged,
            muted,
            bookmarked,
            pinned,
        }
    }
}

/// Convert Status to StatusResponse
#[cfg(test)]
pub fn status_to_response(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    interactions: StatusInteractions,
) -> StatusResponse {
    status_to_response_with_account_stats(
        status,
        account,
        config,
        AccountStats::default(),
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats.
pub fn status_to_response_with_account_stats(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    interactions: StatusInteractions,
) -> StatusResponse {
    status_to_response_with_account_stats_and_remote_count(
        status,
        account,
        config,
        account_stats,
        None,
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats and optional remote count.
pub fn status_to_response_with_account_stats_and_remote_count(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_statuses_count: Option<i32>,
    interactions: StatusInteractions,
) -> StatusResponse {
    let remote_account_stats = remote_statuses_count.map(|statuses_count| RemoteAccountStats {
        statuses_count,
        ..RemoteAccountStats::default()
    });
    status_to_response_with_account_stats_and_remote_stats(
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
    )
}

/// Convert Status to StatusResponse with explicit local account stats and optional remote account stats.
pub fn status_to_response_with_account_stats_and_remote_stats(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
) -> StatusResponse {
    let force_sensitive = remote_account_stats
        .as_ref()
        .map(|stats| stats.force_sensitive)
        .unwrap_or(false);
    status_to_response_with_media(
        status,
        account,
        config,
        account_stats,
        remote_account_stats,
        interactions,
        force_sensitive,
        &[],
    )
}

/// Convert Status to StatusResponse with media attachments
pub fn status_to_response_with_media(
    status: &Status,
    account: &Account,
    config: &AppConfig,
    account_stats: AccountStats,
    remote_account_stats: Option<RemoteAccountStats>,
    interactions: StatusInteractions,
    force_sensitive: bool,
    media_attachments: &[MediaAttachmentResponse],
) -> StatusResponse {
    let base_url = config.server.base_url();
    let account_response = if status.is_local || status.account_address.trim().is_empty() {
        account_to_response_with_stats(account, config, account_stats)
    } else {
        remote_account_to_response(status, config, remote_account_stats.as_ref())
    };
    let text = status_content_to_source_text(&status.content);
    let tags = build_status_tags(&status.content, &base_url);
    let mentions =
        build_status_mentions(&status.content, &base_url, &account.username, &account.id);

    StatusResponse {
        id: status.id.clone(),
        created_at: status.created_at,
        in_reply_to_id: status
            .in_reply_to_uri
            .as_ref()
            .map(|uri| local_status_id_from_uri(uri, config).unwrap_or_else(|| uri.clone())),
        in_reply_to_account_id: None,
        sensitive: status.content_warning.is_some() || force_sensitive,
        spoiler_text: status.content_warning.clone().unwrap_or_default(),
        visibility: status.visibility.to_string(),
        language: status.language.clone(),
        uri: status.uri.clone(),
        url: if status.is_local {
            format!(
                "{}/users/{}/statuses/{}",
                base_url, account.username, status.id
            )
        } else {
            status.uri.clone()
        },
        replies_count: 0,
        reblogs_count: 0,
        favourites_count: 0,
        quotes_count: 0,
        edited_at: None,
        content: status.content.clone(),
        text,
        reblog: status.boost_of_uri.as_deref().map(|uri| {
            Box::new(boost_stub_status(
                uri,
                status,
                account,
                config,
                account_stats,
                None,
            ))
        }),
        application: None,
        account: account_response,
        media_attachments: media_attachments.to_vec(),
        mentions,
        tags,
        emojis: vec![],
        quote: None,
        quote_approval: None,
        card: None,
        poll: None,
        filtered: vec![],
        favourited: interactions.favourited.unwrap_or(false),
        reblogged: interactions.reblogged.unwrap_or(false),
        muted: interactions.muted.unwrap_or(false),
        bookmarked: interactions.bookmarked.unwrap_or(false),
        pinned: interactions.pinned.unwrap_or(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;
    use crate::data::{
        Account, Database, EntityId, PersistedReason, RemoteStatusAttachment, Status,
    };
    use chrono::Utc;
    use tempfile::TempDir;

    fn create_test_config() -> AppConfig {
        AppConfig {
            server: ServerConfig {
                host: "127.0.0.1".to_string(),
                port: 8080,
                domain: "test.example.com".to_string(),
                protocol: "https".to_string(),
                trusted_proxy_ips: Vec::new(),
            },
            database: DatabaseConfig {
                path: "test.db".into(),
                sync: DatabaseSyncConfig::default(),
            },
            storage: StorageConfig {
                media: MediaStorageConfig {
                    bucket: "test-media".to_string(),
                    public_url: "https://media.test.example.com".to_string(),
                },
                backup: BackupStorageConfig {
                    enabled: false,
                    bucket: "test-backup".to_string(),
                    interval_seconds: 86400,
                    retention_count: 7,
                    encryption: BackupEncryptionConfig::default(),
                },
            },
            cloudflare: CloudflareConfig {
                account_id: "test".to_string(),
                r2_access_key_id: "test".to_string(),
                r2_secret_access_key: "test".to_string(),
            },
            auth: AuthConfig {
                username: "testuser".to_string(),
                password: Some("test-password".to_string()),
                session_secret: "secret".to_string(),
                session_max_age: 604800,
            },
            instance: InstanceConfig {
                title: "Test".to_string(),
                description: "Test instance".to_string(),
                contact_email: "test@example.com".to_string(),
            },
            admin: AdminConfig {
                display_name: "Admin".to_string(),
                email: Some("admin@test.example.com".to_string()),
                note: Some("Test administrator".to_string()),
            },
            cache: CacheConfig {
                timeline_max_items: 2000,
                profile_ttl: 86400,
            },
            ui: UiConfig::default(),
            metrics: MetricsConfig::default(),
            logging: LoggingConfig {
                level: "info".to_string(),
                format: "pretty".to_string(),
            },
        }
    }

    #[test]
    fn test_account_to_response() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: Some("Test bio".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: Some("avatar.webp".to_string()),
            header_s3_key: Some("header.webp".to_string()),
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let response = account_to_response(&account, &config);

        assert_eq!(response.id, "123");
        assert_eq!(response.username, "testuser");
        assert_eq!(response.acct, "testuser");
        assert_eq!(response.display_name, "Test User");
        assert_eq!(response.note, "Test bio");
        assert_eq!(response.url, "https://test.example.com/users/testuser");
        assert!(response.avatar.contains("media.test.example.com"));
        assert!(response.avatar.contains("avatar.webp"));
        assert!(!response.locked);
        assert!(!response.bot);
    }

    #[test]
    fn test_status_to_response() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Hello, world!</p>".to_string(),
            content_warning: Some("CW".to_string()),
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response = status_to_response(
            &status,
            &account,
            &config,
            StatusInteractions::new(
                Some(true),
                Some(false),
                Some(false),
                Some(false),
                Some(false),
            ),
        );

        assert_eq!(response.id, "456");
        assert_eq!(response.content, "<p>Hello, world!</p>");
        assert_eq!(response.spoiler_text, "CW");
        assert_eq!(response.visibility, "public");
        assert_eq!(response.language, Some("en".to_string()));
        assert!(response.sensitive);
        assert!(response.favourited);
        assert!(!response.reblogged);
        assert!(!response.muted);
        assert!(!response.bookmarked);
        assert!(!response.pinned);
        assert_eq!(response.account.username, "testuser");
    }

    #[test]
    fn test_status_to_response_remote_account_uses_stable_placeholder_created_at() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let status = Status {
            id: "remote-1".to_string(),
            uri: "https://remote.example/@alice/123".to_string(),
            content: "<p>Remote</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Favourited,
            created_at: chrono::DateTime::parse_from_rfc3339("2020-01-01T00:00:00Z")
                .unwrap()
                .with_timezone(&Utc),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());

        assert_eq!(response.account.acct, "alice@remote.example");
        assert_eq!(
            response.account.created_at,
            chrono::DateTime::from_timestamp(0, 0).unwrap()
        );
        assert!(!response.favourited);
        assert!(!response.reblogged);
        assert!(!response.muted);
        assert!(!response.bookmarked);
        assert!(!response.pinned);
    }

    #[test]
    fn test_status_to_response_remote_account_uses_cached_profile_metadata() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "remote-2".to_string(),
            uri: "https://remote.example/users/alice/statuses/124".to_string(),
            content: "<p>Remote</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "alice@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Timeline,
            created_at: Utc::now(),
            fetched_at: None,
        };
        let remote_stats = RemoteAccountStats {
            followers_count: 7,
            following_count: 8,
            statuses_count: 9,
            force_sensitive: false,
            uri: Some("https://remote.example/users/alice".to_string()),
            display_name: Some("Alice Remote".to_string()),
            note: Some("cached note".to_string()),
            profile_fields_json: Some(
                serde_json::to_string(&vec![serde_json::json!({
                    "name": "Website",
                    "value": "https://alice.example",
                    "verified_at": serde_json::Value::Null,
                })])
                .unwrap(),
            ),
            avatar_url: Some("https://cdn.remote.example/alice.png".to_string()),
            header_url: Some("https://cdn.remote.example/alice-header.png".to_string()),
            locked: true,
            bot: true,
            discoverable: false,
            indexable: false,
        };

        let response = status_to_response_with_account_stats_and_remote_stats(
            &status,
            &account,
            &config,
            AccountStats::default(),
            Some(remote_stats),
            StatusInteractions::default(),
        );

        assert_eq!(response.account.display_name, "Alice Remote");
        assert_eq!(response.account.note, "cached note");
        assert_eq!(response.account.uri, "https://remote.example/users/alice");
        assert_eq!(
            response.account.avatar,
            "https://cdn.remote.example/alice.png"
        );
        assert_eq!(
            response.account.header,
            "https://cdn.remote.example/alice-header.png"
        );
        assert!(response.account.locked);
        assert!(response.account.bot);
        assert!(!response.account.discoverable);
        assert!(!response.account.indexable);
        assert_eq!(response.account.fields[0]["name"], "Website");
        assert_eq!(response.account.followers_count, 7);
        assert_eq!(response.account.following_count, 8);
        assert_eq!(response.account.statuses_count, 9);
    }

    #[test]
    fn test_status_to_response_reblog_stub_does_not_reuse_wrapper_remote_stats() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "remote-boost-1".to_string(),
            uri: "https://remote.example/users/bob/statuses/ann-1".to_string(),
            content: "<p>Boost wrapper</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: Some("https://remote.example/users/alice/statuses/999".to_string()),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Timeline,
            created_at: Utc::now(),
            fetched_at: None,
        };
        let wrapper_stats = RemoteAccountStats {
            followers_count: 42,
            following_count: 24,
            statuses_count: 12,
            force_sensitive: false,
            uri: Some("https://remote.example/users/bob".to_string()),
            display_name: Some("Bob".to_string()),
            note: Some("wrapper".to_string()),
            profile_fields_json: None,
            avatar_url: None,
            header_url: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
        };

        let response = status_to_response_with_account_stats_and_remote_stats(
            &status,
            &account,
            &config,
            AccountStats::default(),
            Some(wrapper_stats),
            StatusInteractions::default(),
        );

        let reblog = response
            .reblog
            .expect("boost should include reblog payload");
        assert_eq!(response.account.username, "bob");
        assert_eq!(reblog.account.username, "alice");
        assert_eq!(reblog.account.followers_count, 0);
        assert_eq!(reblog.account.following_count, 0);
        assert_eq!(reblog.account.statuses_count, 0);
        assert_eq!(reblog.account.note, "");
    }

    #[test]
    fn test_status_to_response_preserves_in_reply_to_uri_as_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: None,
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some("https://remote.example/users/alice/statuses/123".to_string()),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());

        assert_eq!(
            response.in_reply_to_id.as_deref(),
            Some("https://remote.example/users/alice/statuses/123")
        );
    }

    #[test]
    fn test_status_to_response_uses_local_parent_row_id_for_in_reply_to_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: None,
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(
                "https://test.example.com/users/testuser/statuses/local-123".to_string(),
            ),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some("local-123"));
    }

    #[test]
    fn test_status_to_response_keeps_local_activity_uri_when_not_canonical_status_path() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let parent_uri = "https://test.example.com/users/testuser/statuses/local-123/activity";
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(parent_uri.to_string()),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some(parent_uri));
    }

    #[test]
    fn test_status_to_response_does_not_map_mismatched_scheme_local_uri_to_row_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let parent_uri = "http://test.example.com:443/users/testuser/statuses/local-123";
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Reply</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: Some(parent_uri.to_string()),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        assert_eq!(response.in_reply_to_id.as_deref(), Some(parent_uri));
    }

    #[test]
    fn test_status_to_response_populates_reblog_from_boost_uri() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("https://remote.example/users/alice/statuses/999".to_string()),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.id, "https://remote.example/users/alice/statuses/999");
        assert_eq!(
            reblog.uri,
            "https://remote.example/users/alice/statuses/999"
        );
        assert_eq!(reblog.account.acct, "alice@remote.example");
        assert_eq!(reblog.account.username, "alice");
        assert_ne!(reblog.account.username, "testuser");
    }

    #[test]
    fn test_status_to_response_uses_local_boost_row_id_for_reblog_id() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some(
                "https://test.example.com/users/testuser/statuses/local-123".to_string(),
            ),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.id, "local-123");
        assert_eq!(
            reblog.uri,
            "https://test.example.com/users/testuser/statuses/local-123"
        );
        assert_eq!(reblog.account.id, account.id.to_string());
        assert_eq!(reblog.account.acct, account.username);
    }

    #[test]
    fn test_status_to_response_reblog_uses_neutral_placeholder_created_at() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let wrapper_created_at = Utc::now();
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("https://remote.example/users/alice/statuses/999".to_string()),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Reposted,
            created_at: wrapper_created_at,
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");
        let expected = chrono::DateTime::from_timestamp(0, 0)
            .expect("unix epoch timestamp should always be valid");

        assert_eq!(reblog.created_at, expected);
        assert_ne!(reblog.created_at, wrapper_created_at);
    }

    #[test]
    fn test_status_to_response_reblog_preserves_boost_authority_and_scheme() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Boost</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: Some("http://remote.example:8080/users/alice/statuses/999".to_string()),
            quote_of_uri: None,
            persisted_reason: PersistedReason::Reposted,
            created_at: Utc::now(),
            fetched_at: None,
        };

        let response =
            status_to_response(&status, &account, &config, StatusInteractions::default());
        let reblog = response
            .reblog
            .expect("boosted status should include reblog payload");

        assert_eq!(reblog.account.acct, "alice@remote.example:8080");
        assert_eq!(reblog.account.url, "http://remote.example:8080/@alice");
    }

    #[test]
    fn test_status_to_response_with_media_populates_media_attachments() {
        let config = create_test_config();
        let account = Account {
            id: "123".into(),
            username: "testuser".to_string(),
            display_name: None,
            note: None,
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private".to_string(),
            public_key_pem: "public".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let status = Status {
            id: "456".to_string(),
            uri: "https://test.example.com/users/testuser/statuses/456".to_string(),
            content: "<p>Media</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: String::new(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        let media = MediaAttachment {
            id: "media-1".to_string(),
            status_id: Some(status.id.clone()),
            s3_key: "uploads/media-1.webp".to_string(),
            thumbnail_s3_key: Some("uploads/thumb-media-1.webp".to_string()),
            content_type: "image/webp".to_string(),
            file_size: 1024,
            description: Some("alt".to_string()),
            blurhash: Some("LKO2?U%2Tw=w]~RBVZRi};RPxuwH".to_string()),
            width: Some(1200),
            height: Some(800),
            focus_x: Some(0.1),
            focus_y: Some(-0.2),
            created_at: Utc::now(),
        };

        let response = status_to_response_with_media(
            &status,
            &account,
            &config,
            AccountStats::default(),
            None,
            StatusInteractions::default(),
            false,
            &[media_attachment_to_response(&media, &config)],
        );

        assert_eq!(response.media_attachments.len(), 1);
        assert_eq!(response.media_attachments[0].id, "media-1");
        assert_eq!(response.media_attachments[0].media_type, "image");
        assert_eq!(
            response.media_attachments[0].url,
            "https://media.test.example.com/uploads/media-1.webp"
        );
    }

    #[tokio::test]
    async fn build_status_response_with_media_uses_remote_attachment_urls() {
        let temp_dir = TempDir::new().unwrap();
        let db = Database::connect(&temp_dir.path().join("test.db"))
            .await
            .unwrap();
        let config = create_test_config();
        let now = Utc::now();
        let account = Account {
            id: "testuser".to_string(),
            username: "testuser".to_string(),
            display_name: Some("Test User".to_string()),
            note: Some("Test account".to_string()),
            profile_fields_json: None,
            locked: false,
            bot: false,
            discoverable: true,
            indexable: true,
            also_known_as: None,
            moved_to_uri: None,
            avatar_s3_key: None,
            header_s3_key: None,
            private_key_pem: "private-key".to_string(),
            public_key_pem: "public-key".to_string(),
            created_at: now,
            updated_at: now,
        };
        let status = Status {
            id: "https://remote.example/statuses/1".to_string(),
            uri: "https://remote.example/statuses/1".to_string(),
            content: "<p>hello</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "bob@remote.example".to_string(),
            is_local: false,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Timeline,
            created_at: now,
            fetched_at: Some(now),
        };

        db.insert_status(&status).await.unwrap();
        db.replace_remote_status_attachments(
            &status.id,
            &[RemoteStatusAttachment {
                id: EntityId::new_string(),
                status_id: status.id.clone(),
                remote_url: "https://cdn.remote.example/media/original.jpg".to_string(),
                preview_url: Some("https://cdn.remote.example/media/preview.jpg".to_string()),
                content_type: "image/jpeg".to_string(),
                description: Some("remote alt".to_string()),
                blurhash: Some("LEHV6nWB2yk8pyo0adR*.7kCMdnj".to_string()),
                width: Some(1200),
                height: Some(800),
                created_at: now,
            }],
        )
        .await
        .unwrap();

        let response = build_status_response_with_media(
            &db,
            &status,
            &account,
            &config,
            AccountStats::default(),
            Some(RemoteAccountStats::default()),
            StatusInteractions::default(),
            &[],
        )
        .await
        .unwrap();

        assert_eq!(response.media_attachments.len(), 1);
        assert_eq!(
            response.media_attachments[0].url,
            "https://cdn.remote.example/media/original.jpg"
        );
        assert_eq!(
            response.media_attachments[0].preview_url,
            "https://cdn.remote.example/media/preview.jpg"
        );
        assert_eq!(
            response.media_attachments[0].remote_url.as_deref(),
            Some("https://cdn.remote.example/media/original.jpg")
        );
    }
}
