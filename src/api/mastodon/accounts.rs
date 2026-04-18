//! Account endpoints

use axum::{
    body::to_bytes,
    extract::{Path, Query, RawQuery, Request, State},
    http::{
        HeaderMap,
        header::{CONTENT_TYPE, LINK},
    },
    response::{IntoResponse, Json},
};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use futures::stream::{self, StreamExt};
use serde::{Deserialize, de::DeserializeOwned};
use std::{collections::HashSet, convert::Infallible};

use super::federation_delivery::{
    fetch_remote_activity_json, resolve_remote_actor_and_inbox_with_dependencies,
    spawn_best_effort_delivery,
};
use crate::AccountApiState;
use crate::api::dto::AccountResponse;
use crate::auth::CurrentUser;
use crate::data::{Account, CachedProfile};
use crate::error::AppError;
use crate::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, FOLLOWERS_TOTAL, FOLLOWING_TOTAL,
    HTTP_REQUEST_DURATION_SECONDS, HTTP_REQUESTS_TOTAL,
};
use crate::service::TimelineService;

/// Pagination parameters
#[derive(Debug, Deserialize)]
pub struct PaginationParams {
    pub max_id: Option<String>,
    pub since_id: Option<String>,
    pub min_id: Option<String>,
    pub limit: Option<usize>,
}

/// Account statuses timeline query parameters
#[derive(Debug, Deserialize)]
pub struct AccountStatusesParams {
    pub max_id: Option<String>,
    pub since_id: Option<String>,
    pub min_id: Option<String>,
    pub limit: Option<usize>,
    pub exclude_reblogs: Option<bool>,
    pub exclude_replies: Option<bool>,
    pub only_media: Option<bool>,
    pub pinned: Option<bool>,
}

/// Update credentials request
#[derive(Debug, Deserialize)]
pub struct UpdateCredentialsRequest {
    pub display_name: Option<String>,
    pub note: Option<String>,
    pub avatar: Option<String>, // Base64 encoded image
    pub header: Option<String>, // Base64 encoded image
    pub fields_attributes: Option<serde_json::Value>,
    pub moved_to_account_id: Option<String>,
    #[serde(rename = "locked")]
    pub locked: Option<bool>,
    #[serde(rename = "bot")]
    pub bot: Option<bool>,
    #[serde(rename = "discoverable")]
    pub discoverable: Option<bool>,
    pub indexable: Option<bool>,
    #[serde(rename = "hide_collections")]
    pub _hide_collections: Option<bool>,
    pub source: Option<UpdateCredentialsSourceRequest>,
    #[serde(rename = "source[privacy]")]
    pub source_privacy: Option<String>,
    #[serde(rename = "source[sensitive]")]
    pub source_sensitive: Option<bool>,
    #[serde(rename = "source[language]")]
    pub source_language: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateCredentialsSourceRequest {
    pub privacy: Option<String>,
    pub sensitive: Option<bool>,
    pub language: Option<String>,
}

#[derive(Debug)]
enum ProfileImageInput {
    Encoded(String),
    Binary(Vec<u8>),
}

#[derive(Debug, Default)]
struct ParsedUpdateCredentialsRequest {
    display_name: Option<String>,
    note: Option<String>,
    avatar: Option<ProfileImageInput>,
    header: Option<ProfileImageInput>,
    fields_attributes: Option<serde_json::Value>,
    moved_to_account_id: Option<String>,
    locked: Option<bool>,
    bot: Option<bool>,
    discoverable: Option<bool>,
    indexable: Option<bool>,
    source_privacy: Option<String>,
    source_sensitive: Option<bool>,
    source_language: Option<Option<String>>,
}

/// Search query parameters
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    pub limit: Option<usize>,
    pub resolve: Option<bool>,
    pub following: Option<bool>,
    pub offset: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct LookupParams {
    pub acct: String,
}

#[derive(Debug, Default, Deserialize)]
pub struct FollowAccountRequest {
    pub reblogs: Option<bool>,
    pub notify: Option<bool>,
}

const MAX_UPDATE_CREDENTIALS_BODY_BYTES: usize = 12 * 1024 * 1024;
const MAX_PROFILE_IMAGE_UPLOAD_BYTES: usize = 10 * 1024 * 1024;
const MAX_PROFILE_IMAGE_DECODED_BYTES: usize = 64 * 1024 * 1024;
const DEFAULT_POSTING_PRIVACY: &str = "public";
const POSTING_DEFAULT_VISIBILITY_KEY: &str = "posting.default.visibility";
const POSTING_DEFAULT_SENSITIVE_KEY: &str = "posting.default.sensitive";
const POSTING_DEFAULT_LANGUAGE_KEY: &str = "posting.default.language";

#[derive(Debug, Clone)]
struct PostingPreferences {
    privacy: String,
    sensitive: bool,
    language: Option<String>,
}

#[derive(Debug, Clone, Copy)]
struct FollowPreferences {
    reblogs: bool,
    notify: bool,
}

fn normalize_follow_setting_key_suffix(address: &str) -> String {
    address.trim().to_ascii_lowercase()
}

fn follow_reblogs_setting_key(address: &str) -> String {
    format!(
        "follow_preferences.{}.reblogs",
        normalize_follow_setting_key_suffix(address)
    )
}

fn follow_notify_setting_key(address: &str) -> String {
    format!(
        "follow_preferences.{}.notify",
        normalize_follow_setting_key_suffix(address)
    )
}

async fn load_posting_preferences(state: &AccountApiState) -> Result<PostingPreferences, AppError> {
    let privacy = state
        .db
        .get_setting(POSTING_DEFAULT_VISIBILITY_KEY)
        .await?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| DEFAULT_POSTING_PRIVACY.to_string());
    let sensitive = state
        .db
        .get_setting(POSTING_DEFAULT_SENSITIVE_KEY)
        .await?
        .map(|value| matches!(value.trim(), "1" | "true"))
        .unwrap_or(false);
    let language = state
        .db
        .get_setting(POSTING_DEFAULT_LANGUAGE_KEY)
        .await?
        .and_then(|value| {
            let trimmed = value.trim();
            (!trimmed.is_empty()).then(|| trimmed.to_string())
        });

    Ok(PostingPreferences {
        privacy,
        sensitive,
        language,
    })
}

async fn save_posting_preferences(
    state: &AccountApiState,
    privacy: Option<String>,
    sensitive: Option<bool>,
    language: Option<Option<String>>,
) -> Result<(), AppError> {
    if let Some(privacy) = privacy {
        let trimmed = privacy.trim();
        let normalized = match trimmed {
            "public" | "unlisted" | "private" | "direct" => trimmed,
            _ => {
                return Err(AppError::Validation(
                    "source[privacy] must be one of: public, unlisted, private, direct".to_string(),
                ));
            }
        };
        state
            .db
            .set_setting(POSTING_DEFAULT_VISIBILITY_KEY, normalized)
            .await?;
    }

    if let Some(sensitive) = sensitive {
        state
            .db
            .set_setting(
                POSTING_DEFAULT_SENSITIVE_KEY,
                if sensitive { "true" } else { "false" },
            )
            .await?;
    }

    if let Some(language) = language {
        state
            .db
            .set_setting(
                POSTING_DEFAULT_LANGUAGE_KEY,
                language.as_deref().unwrap_or_default(),
            )
            .await?;
    }

    Ok(())
}

async fn load_follow_preferences(
    state: &AccountApiState,
    target_address: &str,
) -> Result<FollowPreferences, AppError> {
    let reblogs = state
        .db
        .get_setting(&follow_reblogs_setting_key(target_address))
        .await?
        .map(|value| !matches!(value.trim(), "0" | "false"))
        .unwrap_or(true);
    let notify = state
        .db
        .get_setting(&follow_notify_setting_key(target_address))
        .await?
        .map(|value| matches!(value.trim(), "1" | "true"))
        .unwrap_or(false);

    Ok(FollowPreferences { reblogs, notify })
}

async fn save_follow_preferences(
    state: &AccountApiState,
    target_address: &str,
    preferences: FollowPreferences,
) -> Result<(), AppError> {
    state
        .db
        .set_setting(
            &follow_reblogs_setting_key(target_address),
            if preferences.reblogs { "true" } else { "false" },
        )
        .await?;
    state
        .db
        .set_setting(
            &follow_notify_setting_key(target_address),
            if preferences.notify { "true" } else { "false" },
        )
        .await?;
    Ok(())
}

fn parse_update_credentials_bool(field: &str, value: &str) -> Result<bool, AppError> {
    match value.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Ok(true),
        "0" | "false" | "off" | "no" => Ok(false),
        _ => Err(AppError::Validation(format!(
            "{field} must be a boolean value"
        ))),
    }
}

fn parse_fields_attribute_key(key: &str) -> Option<(String, String)> {
    let rest = key.strip_prefix("fields_attributes[")?;
    let (index, attribute) = rest.split_once("][")?;
    let attribute = attribute.strip_suffix(']')?;
    if index.is_empty() || attribute.is_empty() {
        return None;
    }
    Some((index.to_string(), attribute.to_string()))
}

fn merge_fields_attribute(
    fields_attributes: &mut Option<serde_json::Value>,
    key: &str,
    value: String,
) -> Result<(), AppError> {
    let Some((index, attribute)) = parse_fields_attribute_key(key) else {
        return Ok(());
    };

    let root = fields_attributes.get_or_insert_with(|| serde_json::json!({}));
    let Some(root_map) = root.as_object_mut() else {
        return Err(AppError::Validation(
            "fields_attributes must be an object, array, or null".to_string(),
        ));
    };
    let entry = root_map
        .entry(index)
        .or_insert_with(|| serde_json::Value::Object(serde_json::Map::new()));
    let Some(entry_map) = entry.as_object_mut() else {
        return Err(AppError::Validation(
            "fields_attributes entries must be objects".to_string(),
        ));
    };
    entry_map.insert(attribute, serde_json::Value::String(value));
    Ok(())
}

fn apply_update_credentials_pair(
    request: &mut ParsedUpdateCredentialsRequest,
    key: &str,
    value: String,
) -> Result<(), AppError> {
    match key {
        "display_name" => request.display_name = Some(value),
        "note" => request.note = Some(value),
        "avatar" => request.avatar = Some(ProfileImageInput::Encoded(value)),
        "header" => request.header = Some(ProfileImageInput::Encoded(value)),
        "moved_to_account_id" => request.moved_to_account_id = Some(value),
        "locked" => request.locked = Some(parse_update_credentials_bool("locked", &value)?),
        "bot" => request.bot = Some(parse_update_credentials_bool("bot", &value)?),
        "discoverable" => {
            request.discoverable = Some(parse_update_credentials_bool("discoverable", &value)?);
        }
        "indexable" => {
            request.indexable = Some(parse_update_credentials_bool("indexable", &value)?);
        }
        "source[privacy]" | "source.privacy" => request.source_privacy = Some(value),
        "source[sensitive]" | "source.sensitive" => {
            request.source_sensitive =
                Some(parse_update_credentials_bool("source[sensitive]", &value)?);
        }
        "source[language]" | "source.language" => {
            request.source_language = Some((!value.trim().is_empty()).then_some(value));
        }
        "fields_attributes" => {
            let parsed = if value.trim().is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::from_str::<serde_json::Value>(&value).map_err(|error| {
                    AppError::Validation(format!("invalid fields_attributes JSON: {error}"))
                })?
            };
            request.fields_attributes = Some(parsed);
        }
        _ if key.starts_with("fields_attributes[") => {
            merge_fields_attribute(&mut request.fields_attributes, key, value)?;
        }
        _ => {}
    }

    Ok(())
}

fn parse_update_credentials_json(body: &[u8]) -> Result<ParsedUpdateCredentialsRequest, AppError> {
    let request: UpdateCredentialsRequest = serde_json::from_slice(body)
        .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")))?;
    Ok(ParsedUpdateCredentialsRequest {
        display_name: request.display_name,
        note: request.note,
        avatar: request.avatar.map(ProfileImageInput::Encoded),
        header: request.header.map(ProfileImageInput::Encoded),
        fields_attributes: request.fields_attributes,
        moved_to_account_id: request.moved_to_account_id,
        locked: request.locked,
        bot: request.bot,
        discoverable: request.discoverable,
        indexable: request.indexable,
        source_privacy: request
            .source
            .as_ref()
            .and_then(|source| source.privacy.clone())
            .or(request.source_privacy),
        source_sensitive: request
            .source
            .as_ref()
            .and_then(|source| source.sensitive)
            .or(request.source_sensitive),
        source_language: request
            .source
            .as_ref()
            .and_then(|source| source.language.clone())
            .map(|value| (!value.trim().is_empty()).then_some(value))
            .or_else(|| {
                request
                    .source_language
                    .map(|value| (!value.trim().is_empty()).then_some(value))
            }),
    })
}

fn parse_update_credentials_form(body: &[u8]) -> Result<ParsedUpdateCredentialsRequest, AppError> {
    let mut request = ParsedUpdateCredentialsRequest::default();
    for (key, value) in url::form_urlencoded::parse(body).into_owned() {
        apply_update_credentials_pair(&mut request, &key, value)?;
    }
    Ok(request)
}

async fn parse_update_credentials_multipart(
    body: axum::body::Bytes,
    content_type: &str,
) -> Result<ParsedUpdateCredentialsRequest, AppError> {
    let boundary = multer::parse_boundary(content_type)
        .map_err(|error| AppError::Validation(format!("invalid multipart boundary: {error}")))?;
    let stream = stream::once(async move { Ok::<_, Infallible>(body) });
    let mut multipart = multer::Multipart::new(stream, boundary);
    let mut request = ParsedUpdateCredentialsRequest::default();

    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|error| AppError::Validation(format!("failed to parse multipart body: {error}")))?
    {
        let Some(name) = field.name().map(ToString::to_string) else {
            continue;
        };

        let is_binary_upload = name == "avatar" || name == "header";
        if is_binary_upload && (field.file_name().is_some() || field.content_type().is_some()) {
            let mut bytes = Vec::new();
            while let Some(chunk) = field.chunk().await.map_err(|error| {
                AppError::Validation(format!("failed to read multipart field `{name}`: {error}"))
            })? {
                if bytes.len() + chunk.len() > MAX_PROFILE_IMAGE_UPLOAD_BYTES {
                    return Err(AppError::Validation(format!(
                        "{name} image exceeds {MAX_PROFILE_IMAGE_UPLOAD_BYTES} bytes"
                    )));
                }
                bytes.extend_from_slice(&chunk);
            }
            match name.as_str() {
                "avatar" => request.avatar = Some(ProfileImageInput::Binary(bytes)),
                "header" => request.header = Some(ProfileImageInput::Binary(bytes)),
                _ => {}
            }
            continue;
        }

        let value = field.text().await.map_err(|error| {
            AppError::Validation(format!("failed to read multipart field `{name}`: {error}"))
        })?;
        apply_update_credentials_pair(&mut request, &name, value)?;
    }

    Ok(request)
}

async fn parse_update_credentials_request(
    headers: &HeaderMap,
    body: axum::body::Bytes,
) -> Result<ParsedUpdateCredentialsRequest, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    if content_type.starts_with("multipart/form-data") {
        return parse_update_credentials_multipart(body, content_type).await;
    }
    if content_type.starts_with("application/x-www-form-urlencoded") {
        return parse_update_credentials_form(&body);
    }
    if content_type.starts_with("application/json") || content_type.is_empty() {
        return parse_update_credentials_json(&body);
    }

    parse_update_credentials_json(&body).or_else(|_| parse_update_credentials_form(&body))
}

fn decode_base64_image_field(field: &str, encoded: &str) -> Result<Vec<u8>, AppError> {
    let trimmed = encoded.trim();
    if trimmed.is_empty() {
        return Err(AppError::Validation(format!(
            "{field} image must not be empty"
        )));
    }

    let payload = if trimmed.starts_with("data:") {
        let (meta, body) = trimmed.split_once(',').ok_or_else(|| {
            AppError::Validation(format!(
                "{field} image must be a base64 data URL or raw base64"
            ))
        })?;
        let meta_lower = meta.to_ascii_lowercase();
        if !meta_lower.contains(";base64") {
            return Err(AppError::Validation(format!(
                "{field} data URL must include ;base64"
            )));
        }
        if !meta_lower.starts_with("data:image/") {
            return Err(AppError::Validation(format!(
                "{field} data URL must use an image MIME type"
            )));
        }
        body
    } else {
        trimmed
    };

    let normalized: String = payload
        .chars()
        .filter(|c| !c.is_ascii_whitespace())
        .collect();
    if normalized.is_empty() {
        return Err(AppError::Validation(format!(
            "{field} image must not be empty"
        )));
    }

    let decoded = BASE64_STANDARD
        .decode(normalized)
        .map_err(|_| AppError::Validation(format!("{field} image is not valid base64")))?;

    normalize_image_bytes_to_webp(field, decoded)
}

fn parse_json_or_form_body<T: DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();

    let parse_json = || {
        serde_json::from_slice(body)
            .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")))
    };
    let parse_form = || {
        serde_urlencoded::from_bytes(body)
            .map_err(|error| AppError::Validation(format!("invalid form body: {error}")))
    };

    if content_type.starts_with("application/x-www-form-urlencoded") {
        return parse_form();
    }
    if content_type.starts_with("application/json") || content_type.is_empty() {
        return parse_json();
    }

    parse_json().or_else(|_| parse_form())
}

fn normalize_image_bytes_to_webp(field: &str, bytes: Vec<u8>) -> Result<Vec<u8>, AppError> {
    if bytes.is_empty() {
        return Err(AppError::Validation(format!(
            "{field} image must not be empty"
        )));
    }
    if bytes.len() > MAX_PROFILE_IMAGE_UPLOAD_BYTES {
        return Err(AppError::Validation(format!(
            "{field} image exceeds {MAX_PROFILE_IMAGE_UPLOAD_BYTES} bytes"
        )));
    }

    let image = image::load_from_memory(&bytes).map_err(|error| {
        AppError::Validation(format!("{field} image must be a supported image: {error}"))
    })?;
    let rgba = image.to_rgba8();
    let width = rgba.width();
    let height = rgba.height();
    if width == 0 || height == 0 {
        return Err(AppError::Validation(format!(
            "{field} image must have non-zero dimensions"
        )));
    }
    if rgba.len() > MAX_PROFILE_IMAGE_DECODED_BYTES {
        return Err(AppError::Validation(format!(
            "{field} image is too large after decoding"
        )));
    }

    let mut encoded = Vec::new();
    image::codecs::webp::WebPEncoder::new_lossless(&mut encoded)
        .encode(
            rgba.as_raw(),
            width,
            height,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|error| AppError::Validation(format!("{field} image encode failed: {error}")))?;

    if !is_valid_webp_container(&encoded) || !is_decodable_webp_image(&encoded) {
        return Err(AppError::Validation(format!(
            "{field} image must contain decodable WebP bytes"
        )));
    }

    Ok(encoded)
}

async fn decode_base64_image_field_blocking(
    field: &'static str,
    encoded: String,
) -> Result<Vec<u8>, AppError> {
    tokio::task::spawn_blocking(move || decode_base64_image_field(field, &encoded))
        .await
        .map_err(|error| AppError::task_join("base64 image decode", error))?
}

async fn normalize_binary_image_field_blocking(
    field: &'static str,
    bytes: Vec<u8>,
) -> Result<Vec<u8>, AppError> {
    tokio::task::spawn_blocking(move || normalize_image_bytes_to_webp(field, bytes))
        .await
        .map_err(|error| AppError::task_join("multipart image decode", error))?
}

fn is_decodable_webp_image(bytes: &[u8]) -> bool {
    use image::ImageDecoder;
    use image::codecs::webp::WebPDecoder;
    use std::io::Cursor;

    let cursor = Cursor::new(bytes);
    let Ok(decoder) = WebPDecoder::new(cursor) else {
        return false;
    };
    let (width, height) = decoder.dimensions();
    if width == 0 || height == 0 {
        return false;
    }

    let total_bytes = decoder.total_bytes();
    if total_bytes == 0 || total_bytes > (64 * 1024 * 1024) as u64 {
        return false;
    }

    let mut output = vec![0_u8; total_bytes as usize];
    decoder.read_image(&mut output).is_ok()
}

fn is_valid_vp8_chunk(payload: &[u8]) -> bool {
    if payload.len() < 10 || payload[3] != 0x9d || payload[4] != 0x01 || payload[5] != 0x2a {
        return false;
    }

    // Keyframe bit must be 0 for streams carrying width/height in the VP8 frame header.
    if payload[0] & 0x01 != 0 {
        return false;
    }

    let first_partition_size =
        ((payload[0] as u32 >> 5) | ((payload[1] as u32) << 3) | ((payload[2] as u32) << 11))
            & 0x7ffff;
    if first_partition_size == 0 {
        return false;
    }
    if first_partition_size as usize > payload.len().saturating_sub(3) {
        return false;
    }

    let width = u16::from_le_bytes([payload[6], payload[7]]) & 0x3fff;
    let height = u16::from_le_bytes([payload[8], payload[9]]) & 0x3fff;
    width > 0 && height > 0
}

fn is_valid_vp8l_chunk(payload: &[u8]) -> bool {
    if payload.len() < 5 || payload[0] != 0x2f {
        return false;
    }

    let packed = u32::from_le_bytes([payload[1], payload[2], payload[3], payload[4]]);
    let version = (packed >> 29) & 0x07;
    if version != 0 {
        return false;
    }

    let width = (packed & 0x3fff) + 1;
    let height = ((packed >> 14) & 0x3fff) + 1;
    width > 0 && height > 0
}

fn anmf_chunk_contains_frame_data(payload: &[u8]) -> bool {
    if payload.len() < 16 {
        return false;
    }

    let mut offset = 16;
    let mut has_frame_data = false;
    while offset + 8 <= payload.len() {
        let chunk_type = &payload[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            payload[offset + 4],
            payload[offset + 5],
            payload[offset + 6],
            payload[offset + 7],
        ]) as usize;
        offset += 8;

        if offset + chunk_size > payload.len() {
            return false;
        }
        let chunk_payload = &payload[offset..offset + chunk_size];

        match chunk_type {
            b"VP8 " => {
                if !is_valid_vp8_chunk(chunk_payload) {
                    return false;
                }
                has_frame_data = true;
            }
            b"VP8L" => {
                if !is_valid_vp8l_chunk(chunk_payload) {
                    return false;
                }
                has_frame_data = true;
            }
            _ => {}
        }

        let padded_size = chunk_size + (chunk_size % 2);
        if offset + padded_size > payload.len() {
            return false;
        }
        offset += padded_size;
    }

    has_frame_data
}

fn is_valid_webp_container(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }

    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    let container_len = riff_size + 8;
    if container_len > bytes.len() {
        return false;
    }
    let bytes = &bytes[..container_len];

    let mut offset = 12;
    let mut has_frame_data = false;
    while offset + 8 <= bytes.len() {
        let chunk_type = &bytes[offset..offset + 4];
        let chunk_size = u32::from_le_bytes([
            bytes[offset + 4],
            bytes[offset + 5],
            bytes[offset + 6],
            bytes[offset + 7],
        ]) as usize;
        offset += 8;

        if offset + chunk_size > bytes.len() {
            return false;
        }
        let payload = &bytes[offset..offset + chunk_size];

        match chunk_type {
            b"VP8 " => {
                if !is_valid_vp8_chunk(payload) {
                    return false;
                }
                has_frame_data = true;
            }
            b"VP8L" => {
                if !is_valid_vp8l_chunk(payload) {
                    return false;
                }
                has_frame_data = true;
            }
            b"VP8X" => {
                if payload.len() != 10 {
                    return false;
                }
            }
            b"ANMF" => {
                if !anmf_chunk_contains_frame_data(payload) {
                    return false;
                }
                has_frame_data = true;
            }
            _ => {}
        }

        let padded_size = chunk_size + (chunk_size % 2);
        if offset + padded_size > bytes.len() {
            return false;
        }
        offset += padded_size;
    }

    has_frame_data
}

pub(crate) fn default_port_for_protocol(protocol: &str) -> Option<u16> {
    if protocol.eq_ignore_ascii_case("http") {
        Some(80)
    } else if protocol.eq_ignore_ascii_case("https") {
        Some(443)
    } else {
        None
    }
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

fn parse_host_and_port(authority: &str) -> Result<(String, Option<u16>), AppError> {
    let parsed = url::Url::parse(&format!("http://{}", authority))
        .map_err(|_| AppError::Validation("Invalid account ID format".to_string()))?;
    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;
    let normalized_host = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();

    Ok((normalized_host, extract_explicit_port(authority)))
}

fn format_authority_host(host: &str) -> String {
    let bare_host = host.trim_start_matches('[').trim_end_matches(']');
    if bare_host.contains(':') {
        format!("[{}]", bare_host)
    } else {
        bare_host.to_string()
    }
}

fn is_same_local_account(target_address: &str, local_address: &str, local_protocol: &str) -> bool {
    let Some((target_user, target_domain)) = target_address.split_once('@') else {
        return false;
    };
    let Some((local_user, local_domain)) = local_address.split_once('@') else {
        return false;
    };

    if !target_user.eq_ignore_ascii_case(local_user) {
        return false;
    }

    let Ok((target_host, target_port)) = parse_host_and_port(target_domain) else {
        return false;
    };
    let Ok((local_host, local_port)) = parse_host_and_port(local_domain) else {
        return false;
    };
    if !target_host.eq_ignore_ascii_case(&local_host) {
        return false;
    }

    let Some(default_port) = default_port_for_protocol(local_protocol) else {
        return target_port == local_port;
    };
    let target_effective_port = target_port.unwrap_or(default_port);
    let local_effective_port = local_port.unwrap_or(default_port);
    target_effective_port == local_effective_port
}

pub(crate) fn normalize_account_address(raw: &str) -> Result<String, AppError> {
    fn normalize_domain(raw: &str) -> Result<String, AppError> {
        let parsed = url::Url::parse(&format!("https://{}", raw))
            .map_err(|_| AppError::Validation("Invalid account ID format".to_string()))?;
        if parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(AppError::Validation(
                "Invalid account ID format".to_string(),
            ));
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;
        let normalized_host = host
            .trim_start_matches('[')
            .trim_end_matches(']')
            .to_ascii_lowercase();
        let authority_host = format_authority_host(&normalized_host);
        let normalized_port = extract_explicit_port(raw);

        Ok(match normalized_port {
            Some(port) => format!("{}:{}", authority_host, port),
            None => authority_host,
        })
    }

    let trimmed = raw.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Err(AppError::Validation(
            "Invalid account ID format".to_string(),
        ));
    }
    let without_leading_at = trimmed.strip_prefix('@').unwrap_or(trimmed);
    let (username, domain) = without_leading_at
        .split_once('@')
        .ok_or_else(|| AppError::Validation("Invalid account ID format".to_string()))?;

    if username.is_empty() || domain.is_empty() || username.contains('@') || domain.contains('@') {
        return Err(AppError::Validation(
            "Invalid account ID format".to_string(),
        ));
    }

    Ok(format!(
        "{}@{}",
        username.to_ascii_lowercase(),
        normalize_domain(domain)?
    ))
}

pub(crate) fn normalize_remote_lookup_account_address(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(parsed) = parse_actor_uri_account_address(trimmed) {
        return normalize_account_address(&parsed).ok();
    }
    normalize_account_address(trimmed).ok()
}

pub(crate) fn parse_actor_uri_account_address(raw: &str) -> Option<String> {
    let parsed = url::Url::parse(raw.trim()).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }

    let normalized_host = parsed
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']')
        .to_ascii_lowercase();
    let authority_host = format_authority_host(&normalized_host);
    let domain = match parsed.port() {
        Some(port) => format!("{authority_host}:{port}"),
        None => authority_host,
    };
    let username = parsed
        .path_segments()
        .and_then(|segments| {
            let collected = segments.collect::<Vec<_>>();
            collected
                .windows(2)
                .find_map(|window| window[0].eq_ignore_ascii_case("users").then_some(window[1]))
                .or_else(|| {
                    collected
                        .iter()
                        .find_map(|segment| segment.strip_prefix('@'))
                })
                .or_else(|| {
                    collected
                        .iter()
                        .rev()
                        .copied()
                        .find(|segment| !segment.is_empty())
                })
        })
        .map(str::to_ascii_lowercase)
        .filter(|value| !value.is_empty())?;

    Some(format!("{}@{}", username, domain))
}

fn actor_uri_placeholder_uses_uri_id(raw: &str) -> bool {
    let Ok(parsed) = url::Url::parse(raw.trim()) else {
        return false;
    };
    let Some(segments) = parsed.path_segments() else {
        return false;
    };
    let collected = segments.collect::<Vec<_>>();
    collected
        .windows(2)
        .any(|window| window[0].eq_ignore_ascii_case("users"))
        || collected.iter().any(|segment| segment.starts_with('@'))
}

fn canonical_remote_account_address(raw: &str) -> Option<String> {
    parse_actor_uri_account_address(raw)
        .and_then(|parsed| normalize_account_address(&parsed).ok())
        .or_else(|| normalize_account_address(raw).ok())
}

fn account_addresses_match_with_default_port(
    left: &str,
    right: &str,
    default_port: Option<u16>,
) -> bool {
    let Ok(left_normalized) = normalize_account_address(left) else {
        return false;
    };
    let Ok(right_normalized) = normalize_account_address(right) else {
        return false;
    };

    if left_normalized == right_normalized {
        return true;
    }

    let Some((left_user, left_domain)) = left_normalized.split_once('@') else {
        return false;
    };
    let Some((right_user, right_domain)) = right_normalized.split_once('@') else {
        return false;
    };
    if !left_user.eq_ignore_ascii_case(right_user) {
        return false;
    }

    let Ok((left_host, left_port)) = parse_host_and_port(left_domain) else {
        return false;
    };
    let Ok((right_host, right_port)) = parse_host_and_port(right_domain) else {
        return false;
    };
    if !left_host.eq_ignore_ascii_case(&right_host) {
        return false;
    }

    match default_port {
        Some(port) => left_port.unwrap_or(port) == right_port.unwrap_or(port),
        None => left_port == right_port,
    }
}

fn remote_account_placeholder_created_at() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(0, 0).expect("unix epoch timestamp should always be valid")
}

fn saturating_count(value: Option<u64>) -> i32 {
    value.unwrap_or(0).min(i32::MAX as u64) as i32
}

fn saturating_count_i64(value: i64) -> i32 {
    value.clamp(0, i64::from(i32::MAX)) as i32
}

async fn observed_statuses_count_for_address(
    db: &crate::data::Database,
    default_port: Option<u16>,
    address: &str,
) -> i32 {
    let Some(normalized) = canonical_remote_account_address(address) else {
        return 0;
    };
    db.count_statuses_by_account_address_with_default_port(&normalized, default_port)
        .await
        .map(saturating_count_i64)
        .unwrap_or(0)
}

fn build_remote_account_response_with_profile(
    normalized_address: &str,
    profile: &CachedProfile,
    config: &crate::config::AppConfig,
    statuses_count: i32,
) -> Option<AccountResponse> {
    let (username, domain) = normalized_address.split_once('@')?;
    let media_url = &config.storage.media.public_url;
    let default_avatar = format!("{}/default-avatar.png", media_url);
    let default_header = format!("{}/default-header.png", media_url);

    let url = Some(profile.uri.clone())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| format!("https://{}/@{}", domain, username));
    let avatar = profile
        .avatar_url
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_avatar.clone());
    let header = profile
        .header_url
        .clone()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| default_header.clone());
    let display_name = profile
        .display_name
        .clone()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| username.to_string());

    Some(AccountResponse {
        id: normalized_address.to_string(),
        username: username.to_string(),
        acct: normalized_address.to_string(),
        uri: profile.uri.clone(),
        display_name,
        locked: profile.locked,
        bot: profile.bot,
        discoverable: profile.discoverable,
        group: false,
        indexable: profile.indexable,
        created_at: profile.fetched_at,
        note: profile.note.clone().unwrap_or_default(),
        url,
        avatar: avatar.clone(),
        avatar_static: avatar,
        header: header.clone(),
        header_static: header,
        followers_count: saturating_count(profile.followers_count),
        following_count: saturating_count(profile.following_count),
        statuses_count,
        last_status_at: None,
        emojis: vec![],
        fields: crate::profile_fields::profile_fields_for_response(
            profile.profile_fields_json.as_deref(),
        ),
        roles: vec![],
        moved: None,
        source: None,
    })
}

pub(crate) fn build_remote_account_placeholder_response(
    address: &str,
    config: &crate::config::AppConfig,
    statuses_count: i32,
) -> Option<AccountResponse> {
    let trimmed = address.trim();
    let (id, username, acct, url) =
        if let Some(parsed_address) = parse_actor_uri_account_address(trimmed) {
            let (username, _domain) = parsed_address.split_once('@')?;
            (
                if actor_uri_placeholder_uses_uri_id(trimmed) {
                    trimmed.to_string()
                } else {
                    parsed_address.clone()
                },
                username.to_string(),
                parsed_address,
                trimmed.to_string(),
            )
        } else if let Some((username, domain)) = trimmed.split_once('@') {
            (
                trimmed.to_string(),
                username.to_ascii_lowercase(),
                trimmed.to_string(),
                format!(
                    "https://{}/@{}",
                    domain.to_ascii_lowercase(),
                    username.to_ascii_lowercase()
                ),
            )
        } else {
            return None;
        };
    let media_url = &config.storage.media.public_url;
    let avatar = format!("{}/default-avatar.png", media_url);
    let header = format!("{}/default-header.png", media_url);

    Some(AccountResponse {
        id,
        username: username.clone(),
        acct,
        uri: url.clone(),
        display_name: username,
        locked: false,
        bot: false,
        discoverable: true,
        group: false,
        indexable: true,
        created_at: remote_account_placeholder_created_at(),
        note: String::new(),
        url,
        avatar: avatar.clone(),
        avatar_static: avatar,
        header: header.clone(),
        header_static: header,
        followers_count: 0,
        following_count: 0,
        statuses_count,
        last_status_at: None,
        emojis: vec![],
        fields: vec![],
        roles: vec![],
        moved: None,
        source: None,
    })
}

pub(crate) async fn resolve_remote_account_response(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: &reqwest::Client,
    raw_address: &str,
) -> Option<AccountResponse> {
    let normalized_address = normalize_remote_lookup_account_address(raw_address)?;

    let mut profile = profile_cache.get(&normalized_address).await;
    if profile.is_none() {
        profile = profile_cache.get_by_uri(raw_address.trim()).await;
    }
    if profile.is_none()
        && resolve_remote_actor_and_inbox_with_dependencies(
            db,
            profile_cache,
            federation_fetch_client,
            &normalized_address,
        )
        .await
        .is_ok()
    {
        profile = profile_cache.get(&normalized_address).await;
    }

    let profile = profile?;
    let statuses_count = observed_statuses_count_for_address(
        db,
        default_port_for_protocol(&config.server.protocol),
        &normalized_address,
    )
    .await;
    build_remote_account_response_with_profile(
        &normalized_address,
        &profile,
        config,
        statuses_count,
    )
}

pub(crate) async fn resolve_cached_remote_account_response(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    raw_address: &str,
) -> Option<AccountResponse> {
    let normalized_address = normalize_remote_lookup_account_address(raw_address)?;
    let profile = if let Some(profile) = profile_cache.get(&normalized_address).await {
        profile
    } else {
        profile_cache.get_by_uri(raw_address.trim()).await?
    };
    let statuses_count = observed_statuses_count_for_address(
        db,
        default_port_for_protocol(&config.server.protocol),
        &normalized_address,
    )
    .await;
    build_remote_account_response_with_profile(
        &normalized_address,
        &profile,
        config,
        statuses_count,
    )
}

pub(crate) async fn resolve_remote_account_response_for_list(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: &reqwest::Client,
    raw_address: &str,
    default_port: Option<u16>,
) -> Option<AccountResponse> {
    if let Some(response) = resolve_remote_account_response_for_list_without_placeholder(
        config,
        db,
        profile_cache,
        federation_fetch_client,
        raw_address,
    )
    .await
    {
        return Some(response);
    }

    let statuses_count = observed_statuses_count_for_address(db, default_port, raw_address).await;
    build_remote_account_placeholder_response(raw_address, config, statuses_count)
}

async fn resolve_remote_account_response_for_list_without_placeholder(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: &reqwest::Client,
    raw_address: &str,
) -> Option<AccountResponse> {
    if let Some(response) =
        resolve_cached_remote_account_response(config, db, profile_cache, raw_address).await
    {
        return Some(response);
    }

    // Keep list endpoints responsive even when remote lookup is slow/unreachable.
    if let Ok(Some(response)) = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        resolve_remote_account_response(
            config,
            db,
            profile_cache,
            federation_fetch_client,
            raw_address,
        ),
    )
    .await
    {
        return Some(response);
    }

    None
}

async fn remote_profile_lock_state(
    state: &AccountApiState,
    target_address: &str,
) -> Result<Option<(Option<String>, bool)>, AppError> {
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    if let Some(profile) = state.profile_cache.get(target_address).await {
        return Ok(Some((Some(profile.uri.clone()), profile.locked)));
    }

    let resolved = resolve_remote_actor_and_inbox_with_dependencies(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        target_address,
    )
    .await;
    if let Ok((actor_uri, _)) = resolved {
        if let Some(profile) = state.profile_cache.get_by_uri(&actor_uri).await {
            return Ok(Some((Some(actor_uri), profile.locked)));
        }
        return Ok(Some((Some(actor_uri), false)));
    }

    if let Some(normalized) = canonical_remote_account_address(target_address)
        && let Some(follow) = state.db.get_follow(&normalized, default_port).await?
    {
        return Ok(Some((follow.actor_uri, false)));
    }

    Ok(None)
}

async fn follow_state_for_target(
    state: &AccountApiState,
    target_address: &str,
) -> Result<(bool, bool), AppError> {
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let has_follow = state
        .db
        .get_follow(target_address, default_port)
        .await?
        .is_some();
    if !has_follow {
        return Ok((false, false));
    }

    let accepted = state
        .db
        .is_follow_accepted(target_address, default_port)
        .await?;
    Ok((accepted, !accepted))
}

async fn relationship_response_for_target(
    state: &AccountApiState,
    requested_id: &str,
    target_address: &str,
) -> Result<crate::api::dto::RelationshipResponse, AppError> {
    use crate::api::dto::RelationshipResponse;

    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let relationship_id = resolve_relationship_id(state, requested_id).await;
    let (following, requested) = follow_state_for_target(state, target_address).await?;
    let followed_by = state
        .db
        .get_follower(target_address, default_port)
        .await?
        .is_some();
    let blocking = state
        .db
        .is_account_blocked(target_address, default_port)
        .await?;
    let muting = state
        .db
        .is_account_muted(target_address, default_port)
        .await?;
    let muting_notifications = state
        .db
        .get_account_mute_notifications(target_address, default_port)
        .await?
        .unwrap_or(false);
    let follow_preferences = load_follow_preferences(state, target_address).await?;

    Ok(RelationshipResponse {
        id: relationship_id,
        following,
        followed_by,
        blocking,
        blocked_by: false,
        muting,
        muting_notifications,
        requested,
        domain_blocking: false,
        showing_reblogs: follow_preferences.reblogs,
        endorsed: false,
        notifying: follow_preferences.notify,
        note: String::new(),
    })
}

async fn resolve_follow_request_requester_address(
    state: &AccountApiState,
    identity: &str,
) -> Result<String, AppError> {
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .resolve_follow_request_requester(identity, default_port)
        .await?
        .ok_or(AppError::NotFound)
}

fn extract_activity_collection_uri(
    actor_document: &serde_json::Value,
    key: &str,
) -> Option<String> {
    actor_document.get(key).and_then(|value| match value {
        serde_json::Value::String(uri) => Some(uri.clone()),
        serde_json::Value::Object(object) => object
            .get("id")
            .or_else(|| object.get("first"))
            .and_then(|inner| inner.as_str())
            .map(ToString::to_string),
        _ => None,
    })
}

fn extract_activity_collection_items(collection: &serde_json::Value) -> Vec<String> {
    fn extract_item_identity(value: &serde_json::Value) -> Option<String> {
        match value {
            serde_json::Value::String(uri) => Some(uri.clone()),
            serde_json::Value::Object(object) => object
                .get("id")
                .or_else(|| object.get("url"))
                .and_then(|inner| inner.as_str())
                .map(ToString::to_string),
            _ => None,
        }
    }

    let items = collection
        .get("orderedItems")
        .or_else(|| collection.get("items"))
        .and_then(|value| value.as_array())
        .cloned()
        .unwrap_or_default();
    items
        .iter()
        .filter_map(extract_item_identity)
        .collect::<Vec<_>>()
}

fn extract_activity_collection_page_uri(
    collection: &serde_json::Value,
    key: &str,
) -> Option<String> {
    collection.get(key).and_then(|value| match value {
        serde_json::Value::String(uri) => Some(uri.clone()),
        serde_json::Value::Object(object) => object
            .get("id")
            .or_else(|| object.get("url"))
            .and_then(|inner| inner.as_str())
            .map(ToString::to_string),
        _ => None,
    })
}

async fn fetch_remote_collection_identities(
    state: &AccountApiState,
    target_address: &str,
    collection_key: &str,
    limit: usize,
) -> Result<Vec<String>, AppError> {
    let Ok(Some((actor_uri, _locked))) = remote_profile_lock_state(state, target_address).await
    else {
        return Err(AppError::NotFound);
    };
    let Some(actor_uri) = actor_uri else {
        return Err(AppError::NotFound);
    };

    let actor_document =
        fetch_remote_activity_json(state.federation_fetch_client.as_ref(), &actor_uri).await;
    let Ok(actor_document) = actor_document else {
        return Ok(Vec::new());
    };
    let Some(collection_uri) = extract_activity_collection_uri(&actor_document, collection_key)
    else {
        return Ok(Vec::new());
    };

    let mut seen_pages = HashSet::new();
    let mut seen_identities = HashSet::new();
    let mut identities = Vec::new();
    let initial_page =
        fetch_remote_activity_json(state.federation_fetch_client.as_ref(), &collection_uri).await;
    let Ok(mut page) = initial_page else {
        return Ok(Vec::new());
    };

    if extract_activity_collection_items(&page).is_empty()
        && let Some(first_uri) = extract_activity_collection_page_uri(&page, "first")
    {
        match fetch_remote_activity_json(state.federation_fetch_client.as_ref(), &first_uri).await {
            Ok(first_page) => page = first_page,
            Err(_) => return Ok(Vec::new()),
        }
    }

    loop {
        let page_id = page
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or(&collection_uri)
            .to_string();
        if !seen_pages.insert(page_id) {
            break;
        }

        for identity in extract_activity_collection_items(&page) {
            if seen_identities.insert(identity.clone()) {
                identities.push(identity);
                if identities.len() >= limit {
                    return Ok(identities);
                }
            }
        }

        let Some(next_uri) = extract_activity_collection_page_uri(&page, "next") else {
            break;
        };
        let Ok(next_page) =
            fetch_remote_activity_json(state.federation_fetch_client.as_ref(), &next_uri).await
        else {
            break;
        };
        page = next_page;
    }

    Ok(identities)
}

async fn resolve_remote_collection_accounts(
    state: &AccountApiState,
    target_address: &str,
    params: &PaginationParams,
    collection_key: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let limit = params.limit.unwrap_or(40).min(80);
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let fetch_limit = limit.max(120).min(400);
    let identities =
        fetch_remote_collection_identities(state, target_address, collection_key, fetch_limit)
            .await?;

    let mut resolved = Vec::new();
    for identity in identities {
        if let Some(response) = resolve_remote_account_response_for_list(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            state.federation_fetch_client.as_ref(),
            &identity,
            default_port,
        )
        .await
        .and_then(|response| serde_json::to_value(response).ok())
        {
            resolved.push(response);
        }
    }

    Ok(paginate_account_response_values(resolved, params, limit))
}

async fn resolve_moved_account_response(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: Option<&reqwest::Client>,
    moved_to_uri: &str,
) -> Option<Box<AccountResponse>> {
    let response = if let Some(response) =
        resolve_cached_remote_account_response(config, db, profile_cache, moved_to_uri).await
    {
        response
    } else if let Some(client) = federation_fetch_client {
        if let Some(response) =
            resolve_remote_account_response(config, db, profile_cache, client, moved_to_uri).await
        {
            response
        } else {
            let default_port = default_port_for_protocol(&config.server.protocol);
            let statuses_count =
                observed_statuses_count_for_address(db, default_port, moved_to_uri).await;
            build_remote_account_placeholder_response(moved_to_uri, config, statuses_count)?
        }
    } else {
        let default_port = default_port_for_protocol(&config.server.protocol);
        let statuses_count =
            observed_statuses_count_for_address(db, default_port, moved_to_uri).await;
        build_remote_account_placeholder_response(moved_to_uri, config, statuses_count)?
    };
    Some(Box::new(response))
}

async fn populate_local_account_compat_fields(
    state: &AccountApiState,
    account: &Account,
    response: &mut AccountResponse,
) {
    if let Some(moved_to_uri) = account
        .moved_to_uri
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        response.moved = resolve_moved_account_response(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            Some(state.federation_fetch_client.as_ref()),
            moved_to_uri,
        )
        .await;
    }
}

fn local_account_matches_identity(
    account: &Account,
    config: &crate::config::AppConfig,
    identity: &str,
) -> bool {
    let trimmed = identity.trim();
    if trimmed.is_empty() {
        return false;
    }

    if account.id.as_str() == trimmed {
        return true;
    }

    let normalized_username = account.username.trim();
    if trimmed
        .trim_start_matches('@')
        .eq_ignore_ascii_case(normalized_username)
    {
        return true;
    }

    let local_address = format!("{}@{}", account.username, config.server.domain);
    if normalize_account_address(trimmed).ok().as_deref()
        == normalize_account_address(&local_address).ok().as_deref()
    {
        return true;
    }

    let actor_uri = format!("{}/users/{}", config.server.base_url(), account.username);
    trimmed.eq_ignore_ascii_case(actor_uri.as_str())
}

pub(crate) async fn resolve_account_response_for_identity(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: Option<&reqwest::Client>,
    raw_identity: &str,
) -> Option<AccountResponse> {
    let account = db.get_account().await.ok().flatten()?;
    if local_account_matches_identity(&account, config, raw_identity) {
        let stats = crate::api::load_local_account_stats(db).await.ok()?;
        let mut response = crate::api::account_to_response_with_stats(&account, config, stats);
        if let Some(moved_to_uri) = account
            .moved_to_uri
            .as_deref()
            .filter(|value| !value.trim().is_empty())
        {
            response.moved = resolve_moved_account_response(
                config,
                db,
                profile_cache,
                federation_fetch_client,
                moved_to_uri,
            )
            .await;
        }
        return Some(response);
    }

    if let Some(response) =
        resolve_cached_remote_account_response(config, db, profile_cache, raw_identity).await
    {
        return Some(response);
    }

    if let Some(client) = federation_fetch_client {
        if let Some(response) =
            resolve_remote_account_response(config, db, profile_cache, client, raw_identity).await
        {
            return Some(response);
        }
    }

    let default_port = default_port_for_protocol(&config.server.protocol);
    let statuses_count = observed_statuses_count_for_address(db, default_port, raw_identity).await;
    build_remote_account_placeholder_response(raw_identity, config, statuses_count)
}

fn list_entry_identity(actor_uri: Option<&str>, address: &str) -> Option<String> {
    let actor_uri = actor_uri
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    if actor_uri.is_some() {
        return actor_uri;
    }

    let trimmed = address.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(trimmed.to_string())
}

fn list_entry_dedup_key(identity: &str) -> Option<String> {
    normalize_remote_lookup_account_address(identity).or_else(|| {
        let trimmed = identity.trim();
        if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
            Some(trimmed.to_ascii_lowercase())
        } else {
            None
        }
    })
}

fn contains_equivalent_address(
    seen_addresses: &[String],
    candidate: &str,
    default_port: Option<u16>,
) -> bool {
    seen_addresses
        .iter()
        .any(|seen| account_addresses_match_with_default_port(seen, candidate, default_port))
}

fn paginate_account_response_values(
    mut accounts: Vec<serde_json::Value>,
    params: &PaginationParams,
    limit: usize,
) -> Vec<serde_json::Value> {
    let max_id = params
        .max_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let min_id = params
        .min_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let since_id = params
        .since_id
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());

    accounts.sort_by(|left, right| {
        let left_id = left
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        let right_id = right
            .get("id")
            .and_then(|value| value.as_str())
            .unwrap_or_default();
        right_id.cmp(left_id)
    });
    accounts.dedup_by(|left, right| {
        left.get("id").and_then(|value| value.as_str())
            == right.get("id").and_then(|value| value.as_str())
    });
    let mut accounts = accounts
        .into_iter()
        .filter(|account| {
            let id = account
                .get("id")
                .and_then(|value| value.as_str())
                .unwrap_or_default();
            max_id.map(|cursor| id < cursor).unwrap_or(true)
                && min_id.map(|cursor| id > cursor).unwrap_or(true)
                && since_id.map(|cursor| id > cursor).unwrap_or(true)
        })
        .take(limit)
        .collect::<Vec<_>>();
    if min_id.is_some() {
        accounts.reverse();
    }
    accounts
}

fn account_collection_link_header(
    path: &str,
    limit: usize,
    first_id: Option<&str>,
    last_id: Option<&str>,
) -> Option<String> {
    let build_path = |cursor_key: &str, cursor_value: &str| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        serializer.append_pair("limit", &limit.to_string());
        serializer.append_pair(cursor_key, cursor_value);
        format!("{path}?{}", serializer.finish())
    };

    let mut links = Vec::new();
    if let Some(last_id) = last_id.filter(|value| !value.is_empty()) {
        links.push(format!("<{}>; rel=\"next\"", build_path("max_id", last_id)));
    }
    if let Some(first_id) = first_id.filter(|value| !value.is_empty()) {
        links.push(format!(
            "<{}>; rel=\"prev\"",
            build_path("min_id", first_id)
        ));
    }
    (!links.is_empty()).then(|| links.join(", "))
}

fn canonical_account_identity(acct: &str, local_domain: &str) -> String {
    let normalized_acct = acct.trim().trim_start_matches('@').to_ascii_lowercase();
    if normalized_acct.contains('@') {
        normalized_acct
    } else {
        format!(
            "{normalized_acct}@{}",
            local_domain.trim().to_ascii_lowercase()
        )
    }
}

fn build_delivery(
    state: &AccountApiState,
    account: &Account,
) -> crate::federation::ActivityDelivery {
    crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    )
    .with_media_storage(state.storage.clone())
}

async fn resolve_remote_actor_and_inbox(
    state: &AccountApiState,
    address: &str,
) -> Result<(String, String), AppError> {
    resolve_remote_actor_and_inbox_with_dependencies(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        address,
    )
    .await
}

async fn normalize_moved_to_account_uri(
    state: &AccountApiState,
    moved_to_account_id: &str,
) -> Result<Option<String>, AppError> {
    let trimmed = moved_to_account_id.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }

    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        return Ok(Some(trimmed.trim_end_matches('/').to_string()));
    }

    let (actor_uri, _) = resolve_remote_actor_and_inbox(state, trimmed).await?;
    Ok(Some(actor_uri))
}

async fn account_source_payload(
    state: &AccountApiState,
    account: &crate::data::Account,
) -> Result<serde_json::Value, AppError> {
    let follow_requests_count = state
        .db
        .get_follow_request_addresses(500)
        .await
        .map(|requests| requests.len())
        .unwrap_or(0);
    let preferences = load_posting_preferences(state).await?;
    Ok(serde_json::json!({
        "note": account.note.clone().unwrap_or_default(),
        "fields": crate::profile_fields::profile_fields_for_source(
            account.profile_fields_json.as_deref(),
        ),
        "privacy": preferences.privacy,
        "sensitive": preferences.sensitive,
        "language": preferences.language,
        "follow_requests_count": follow_requests_count,
    }))
}

async fn resolve_remote_actor_and_inbox_with_hint(
    state: &AccountApiState,
    address: &str,
    actor_uri_hint: Option<&str>,
) -> Result<(String, String), AppError> {
    if let Some(actor_uri) = actor_uri_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        if let Some(profile) = state.profile_cache.get_by_uri(actor_uri).await {
            if !profile.address.eq_ignore_ascii_case(address) {
                let mut aliased = (*profile).clone();
                aliased.address = address.to_string();
                aliased.uri = actor_uri.to_string();
                state.profile_cache.insert(aliased).await;
            }
            return Ok((actor_uri.to_string(), profile.inbox_uri.clone()));
        }
    }

    if let Some(profile) = state.profile_cache.get(address).await {
        return Ok((profile.uri.clone(), profile.inbox_uri.clone()));
    }

    resolve_remote_actor_and_inbox(state, address).await
}

async fn resolve_remote_actor_and_inbox_with_stored_hints(
    state: &AccountApiState,
    address: &str,
    actor_uri_hint: Option<&str>,
    inbox_uri_hint: Option<&str>,
) -> Result<(String, String), AppError> {
    let actor_uri_hint = actor_uri_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let inbox_uri_hint = inbox_uri_hint
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    if let (Some(actor_uri), Some(inbox_uri)) = (actor_uri_hint.clone(), inbox_uri_hint.clone()) {
        return Ok((actor_uri, inbox_uri));
    }

    resolve_remote_actor_and_inbox_with_hint(state, address, actor_uri_hint.as_deref()).await
}

async fn resolve_target_address(state: &AccountApiState, id: &str) -> Result<String, AppError> {
    if id.starts_with("http://") || id.starts_with("https://") {
        if let Some(address) = parse_actor_uri_account_address(id) {
            return normalize_account_address(&address);
        }
        return Err(AppError::Validation(
            "Invalid account ID format".to_string(),
        ));
    }

    if id.contains('@') {
        return normalize_account_address(id);
    }

    if let Some(account) = state.db.get_account().await?
        && account.id.as_str() == id
    {
        return normalize_account_address(&format!(
            "{}@{}",
            account.username, state.config.server.domain
        ));
    }

    Err(AppError::Validation(
        "Invalid account ID format".to_string(),
    ))
}

async fn resolve_relationship_id(state: &AccountApiState, requested_id: &str) -> String {
    if let Some(response) = resolve_account_response_for_identity(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        Some(state.federation_fetch_client.as_ref()),
        requested_id,
    )
    .await
    {
        return response.id;
    }

    if let Ok(address) = resolve_target_address(state, requested_id).await {
        return address;
    }

    requested_id.to_string()
}

fn build_remote_account_stub(
    config: &crate::config::AppConfig,
    raw_identity: &str,
    statuses_count: i32,
) -> serde_json::Value {
    if let Some(response) =
        build_remote_account_placeholder_response(raw_identity, config, statuses_count)
    {
        return serde_json::to_value(response).unwrap_or_else(|_| {
            serde_json::json!({
                "id": raw_identity,
                "username": raw_identity,
                "acct": raw_identity,
                "uri": raw_identity,
                "display_name": raw_identity,
                "locked": false,
                "bot": false,
                "discoverable": true,
                "group": false,
                "indexable": true,
                "created_at": remote_account_placeholder_created_at(),
                "note": "",
                "url": raw_identity,
                "avatar": format!("{}/default-avatar.png", config.storage.media.public_url),
                "avatar_static": format!("{}/default-avatar.png", config.storage.media.public_url),
                "header": format!("{}/default-header.png", config.storage.media.public_url),
                "header_static": format!("{}/default-header.png", config.storage.media.public_url),
                "followers_count": 0,
                "following_count": 0,
                "statuses_count": statuses_count,
                "last_status_at": serde_json::Value::Null,
                "emojis": [],
                "fields": [],
                "roles": [],
            })
        });
    }

    serde_json::json!({
        "id": raw_identity,
        "username": raw_identity,
        "acct": raw_identity,
        "uri": raw_identity,
        "display_name": raw_identity,
        "locked": false,
        "bot": false,
        "discoverable": true,
        "group": false,
        "indexable": true,
        "created_at": remote_account_placeholder_created_at(),
        "note": "",
        "url": raw_identity,
        "avatar": format!("{}/default-avatar.png", config.storage.media.public_url),
        "avatar_static": format!("{}/default-avatar.png", config.storage.media.public_url),
        "header": format!("{}/default-header.png", config.storage.media.public_url),
        "header_static": format!("{}/default-header.png", config.storage.media.public_url),
        "followers_count": 0,
        "following_count": 0,
        "statuses_count": statuses_count,
        "last_status_at": serde_json::Value::Null,
        "emojis": [],
        "fields": [],
        "roles": [],
    })
}

async fn resolve_remote_account_or_stub(
    state: AccountApiState,
    address: String,
    default_port: Option<u16>,
) -> serde_json::Value {
    let statuses_count =
        observed_statuses_count_for_address(state.db.as_ref(), default_port, &address).await;
    if let Some(response) = resolve_remote_account_response_for_list_without_placeholder(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        &address,
    )
    .await
    {
        return serde_json::to_value(response).unwrap_or_else(|_| {
            build_remote_account_stub(state.config.as_ref(), &address, statuses_count)
        });
    }

    let mut stub = build_remote_account_stub(state.config.as_ref(), &address, statuses_count);
    if url::Url::parse(&address)
        .ok()
        .is_some_and(|parsed| matches!(parsed.scheme(), "http" | "https"))
    {
        stub["id"] = serde_json::Value::String(address);
    }
    stub
}

pub(crate) async fn resolve_remote_account_value_for_list(
    config: &crate::config::AppConfig,
    db: &crate::data::Database,
    profile_cache: &crate::data::ProfileCache,
    federation_fetch_client: &reqwest::Client,
    raw_identity: &str,
    default_port: Option<u16>,
) -> serde_json::Value {
    let statuses_count = observed_statuses_count_for_address(db, default_port, raw_identity).await;
    if let Some(response) = resolve_remote_account_response_for_list(
        config,
        db,
        profile_cache,
        federation_fetch_client,
        raw_identity,
        default_port,
    )
    .await
    {
        return serde_json::to_value(response)
            .unwrap_or_else(|_| build_remote_account_stub(config, raw_identity, statuses_count));
    }

    build_remote_account_stub(config, raw_identity, statuses_count)
}

/// GET /api/v1/accounts/verify_credentials
pub async fn verify_credentials(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["GET", "/api/v1/accounts/verify_credentials"])
        .start_timer();

    // Get the account from database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "accounts"])
        .start_timer();
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "accounts"])
        .inc();
    db_timer.observe_duration();

    // Get counts
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "followers"])
        .start_timer();
    let followers_count = state.db.count_follower_addresses().await? as i32;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "followers"])
        .inc();
    db_timer.observe_duration();

    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "follows"])
        .start_timer();
    let following_count = state.db.count_follow_addresses().await? as i32;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "follows"])
        .inc();
    db_timer.observe_duration();

    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["SELECT", "statuses"])
        .start_timer();
    let statuses_count = state.db.count_local_statuses().await? as i32;
    DB_QUERIES_TOTAL
        .with_label_values(&["SELECT", "statuses"])
        .inc();
    db_timer.observe_duration();

    // Convert to API response
    let mut response = crate::api::account_to_response_with_stats(
        &account,
        &state.config,
        crate::api::AccountStats {
            followers_count,
            following_count,
            statuses_count,
        },
    );
    populate_local_account_compat_fields(&state, &account, &mut response).await;

    // Update metrics
    FOLLOWERS_TOTAL.set(followers_count as i64);
    FOLLOWING_TOTAL.set(following_count as i64);

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["GET", "/api/v1/accounts/verify_credentials", "200"])
        .inc();

    let moved_value = response
        .moved
        .as_ref()
        .map(|moved| serde_json::to_value(moved).unwrap());
    let mut value = serde_json::to_value(response).unwrap();
    if let Some(obj) = value.as_object_mut() {
        if let Some(moved) = moved_value {
            obj.insert("moved".to_string(), moved);
        }
        obj.insert(
            "source".to_string(),
            account_source_payload(&state, &account).await?,
        );
    }
    Ok(Json(value))
}

/// GET /api/v1/preferences
pub async fn preferences(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let preferences = load_posting_preferences(&state).await?;
    Ok(Json(serde_json::json!({
        "posting:default:visibility": preferences.privacy,
        "posting:default:sensitive": preferences.sensitive,
        "posting:default:language": preferences.language,
        "posting:default:quote_policy": "public",
        "posting:default:privacy": preferences.privacy,
        "posting:default:media_sensitive": preferences.sensitive,
        "posting:default:content_type": "text/plain",
        "reading:expand:media": "default",
        "reading:expand:spoilers": false,
        "reading:autoplay:gifs": true,
        "reading:display:media": "default",
        "reading:display:expand_media": "default",
        "reading:display:expand_spoilers": false,
        "notifications:follow": true,
        "notifications:favourite": true,
        "notifications:reblog": true,
        "notifications:mention": true,
        "notifications:poll": true,
        "web:theme": "default",
    })))
}

/// PATCH /api/v1/accounts/update_credentials
pub async fn update_credentials(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let account_service =
        crate::service::AccountService::new(state.db.clone(), state.storage.clone());

    let (parts, body) = request.into_parts();
    let body = to_bytes(body, MAX_UPDATE_CREDENTIALS_BODY_BYTES)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let req = parse_update_credentials_request(&parts.headers, body).await?;

    let ParsedUpdateCredentialsRequest {
        display_name,
        note,
        avatar,
        header,
        fields_attributes,
        moved_to_account_id,
        locked,
        bot,
        discoverable,
        indexable,
        source_privacy,
        source_sensitive,
        source_language,
    } = req;

    let profile_fields_json =
        crate::profile_fields::normalize_profile_fields_request(fields_attributes.as_ref())?;

    let normalized_moved_to_uri = match moved_to_account_id.as_deref() {
        Some(value) => normalize_moved_to_account_uri(&state, value).await?,
        None => None,
    };
    save_posting_preferences(&state, source_privacy, source_sensitive, source_language).await?;

    let avatar_bytes = match avatar {
        Some(ProfileImageInput::Encoded(encoded)) => {
            Some(decode_base64_image_field_blocking("avatar", encoded).await?)
        }
        Some(ProfileImageInput::Binary(bytes)) => {
            Some(normalize_binary_image_field_blocking("avatar", bytes).await?)
        }
        None => None,
    };
    let header_bytes = match header {
        Some(ProfileImageInput::Encoded(encoded)) => {
            Some(decode_base64_image_field_blocking("header", encoded).await?)
        }
        Some(ProfileImageInput::Binary(bytes)) => {
            Some(normalize_binary_image_field_blocking("header", bytes).await?)
        }
        None => None,
    };

    let mut account = account_service
        .update_credentials(
            display_name,
            note,
            profile_fields_json,
            locked,
            bot,
            discoverable,
            indexable,
            avatar_bytes,
            header_bytes,
        )
        .await?;

    if moved_to_account_id.is_some() {
        let local_actor_uri = format!(
            "{}/users/{}",
            state.config.server.base_url(),
            account.username
        );
        if normalized_moved_to_uri.as_deref() == Some(local_actor_uri.as_str()) {
            return Err(AppError::Validation(
                "moved_to_account_id must not point to the local actor".to_string(),
            ));
        }

        let updated = state
            .db
            .patch_account_migration(
                &account.id,
                None,
                Some(normalized_moved_to_uri.as_deref()),
                chrono::Utc::now(),
            )
            .await?;
        if !updated {
            return Err(AppError::NotFound);
        }

        if let Some(moved_to_uri) = normalized_moved_to_uri.as_deref() {
            let follower_inboxes = state.db.get_follower_inboxes().await?;
            if !follower_inboxes.is_empty() {
                let delivery = build_delivery(&state, &account);
                let _ = delivery
                    .queue_move(state.db.as_ref(), moved_to_uri, follower_inboxes)
                    .await;
            }
        }

        account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    }

    let follower_inboxes = state
        .db
        .get_follower_inboxes()
        .await
        .unwrap_or_else(|error| {
            tracing::warn!(
                %error,
                "Skipping follower inbox prefetch for outbound actor Update delivery"
            );
            Vec::new()
        });
    if !follower_inboxes.is_empty() {
        let follower_actor_uris = state
            .db
            .get_all_followers()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|follower| follower.actor_uri)
            .collect::<Vec<_>>();
        let actor_object = crate::api::activitypub::build_local_actor_document(
            state.storage.as_ref(),
            &state.config.server.base_url(),
            &account,
        );
        let delivery = build_delivery(&state, &account);
        let _ = delivery
            .queue_update_actor(
                state.db.as_ref(),
                actor_object,
                follower_inboxes,
                &follower_actor_uris,
            )
            .await;
    }

    // Get counts
    let followers_count = state.db.count_follower_addresses().await? as i32;
    let following_count = state.db.count_follow_addresses().await? as i32;
    let statuses_count = state.db.count_local_statuses().await? as i32;

    // Return updated account
    let mut response = crate::api::account_to_response_with_stats(
        &account,
        &state.config,
        crate::api::AccountStats {
            followers_count,
            following_count,
            statuses_count,
        },
    );
    populate_local_account_compat_fields(&state, &account, &mut response).await;

    let moved_value = response
        .moved
        .as_ref()
        .map(|moved| serde_json::to_value(moved).unwrap());
    let mut value = serde_json::to_value(response).unwrap();
    if let Some(obj) = value.as_object_mut() {
        if let Some(moved) = moved_value {
            obj.insert("moved".to_string(), moved);
        }
        obj.insert(
            "source".to_string(),
            account_source_payload(&state, &account).await?,
        );
    }
    Ok(Json(value))
}

/// GET /api/v1/accounts/:id
pub async fn get_account(
    State(state): State<AccountApiState>,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let response = if local_account_matches_identity(&account, state.config.as_ref(), &id) {
        let stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
        let mut response =
            crate::api::account_to_response_with_stats(&account, &state.config, stats);
        populate_local_account_compat_fields(&state, &account, &mut response).await;
        response
    } else if let Some(response) = resolve_cached_remote_account_response(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &id,
    )
    .await
    {
        response
    } else if let Some(response) = resolve_remote_account_response(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        &id,
    )
    .await
    {
        response
    } else {
        return Err(AppError::NotFound);
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

fn is_public_account_status_visibility(visibility: crate::data::StatusVisibility) -> bool {
    matches!(
        visibility,
        crate::data::StatusVisibility::Public | crate::data::StatusVisibility::Unlisted
    )
}

/// GET /api/v1/accounts/:id/statuses
pub async fn account_statuses(
    State(state): State<AccountApiState>,
    Path(id): Path<String>,
    Query(params): Query<AccountStatusesParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let account_stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let requested_identity = if local_account_matches_identity(&account, state.config.as_ref(), &id)
    {
        None
    } else {
        Some(resolve_target_address(&state, &id).await?)
    };

    let limit = params.limit.unwrap_or(20).min(40);
    let only_pinned = params.pinned.unwrap_or(false);
    let exclude_reblogs = params.exclude_reblogs.unwrap_or(false);
    let exclude_replies = params.exclude_replies.unwrap_or(false);
    let only_media = params.only_media.unwrap_or(false);
    let lower_bound_id = params.min_id.as_deref().or(params.since_id.as_deref());
    let timeline_service = TimelineService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.profile_cache.clone(),
    );
    let mut timeline_items = timeline_service
        .account_timeline(
            requested_identity.as_deref(),
            default_port,
            limit,
            params.max_id.as_deref(),
            lower_bound_id,
            only_media,
            exclude_replies,
            exclude_reblogs,
            only_pinned,
        )
        .await?;
    if params.min_id.is_some() {
        timeline_items.reverse();
    }
    let timeline_statuses: Vec<_> = timeline_items
        .iter()
        .map(|item| item.status.clone())
        .collect();
    let remote_account_stats = crate::api::load_remote_account_stats_map(
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        &state.config.server.protocol,
        &timeline_statuses,
    )
    .await
    .unwrap_or_default();
    let mut responses = Vec::with_capacity(limit);
    for item in timeline_items {
        if !is_public_account_status_visibility(item.status.visibility) {
            continue;
        }
        let is_pinned = state.db.is_status_pinned(&item.status.id).await?;
        let remote_stats = remote_account_stats
            .get(item.status.account_address.trim())
            .cloned();
        let response = crate::api::build_status_response_with_account_stats_and_remote_stats(
            state.db.as_ref(),
            &item.status,
            &account,
            &state.config,
            account_stats,
            remote_stats,
            crate::api::StatusInteractions::new(
                Some(item.favourited),
                Some(item.reblogged),
                None,
                Some(item.bookmarked),
                Some(is_pinned),
            ),
        )
        .await?;
        responses.push(serde_json::to_value(response).unwrap());
    }

    Ok(Json(responses))
}

/// GET /api/v1/accounts/:id/followers
pub async fn get_account_followers(
    State(state): State<AccountApiState>,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let limit = params.limit.unwrap_or(40).min(80);
    let local_requested = local_account_matches_identity(&account, state.config.as_ref(), &id);
    if !local_requested {
        let target_address = resolve_target_address(&state, &id).await?;
        let followers =
            resolve_remote_collection_accounts(&state, &target_address, &params, "followers")
                .await?;
        let first_id = followers
            .first()
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str());
        let last_id = followers
            .last()
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str());
        let mut headers = HeaderMap::new();
        if let Some(link) = account_collection_link_header(
            &format!("/api/v1/accounts/{id}/followers"),
            limit,
            first_id,
            last_id,
        ) {
            headers.insert(
                LINK,
                link.parse()
                    .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
            );
        }
        return Ok((headers, Json(followers)));
    }

    let follower_entries = state.db.get_all_followers().await?;
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let mut unique_keys: Vec<String> = Vec::new();
    let mut identities: Vec<String> = Vec::new();
    let mut followers = Vec::new();

    for follower in follower_entries {
        let Some(candidate) =
            list_entry_identity(follower.actor_uri.as_deref(), &follower.follower_address)
        else {
            continue;
        };
        let Some(dedup_key) = list_entry_dedup_key(&candidate) else {
            continue;
        };

        let is_duplicate = if normalize_account_address(&dedup_key).is_ok() {
            contains_equivalent_address(&unique_keys, &dedup_key, default_port)
        } else {
            unique_keys
                .iter()
                .any(|seen| seen.eq_ignore_ascii_case(&dedup_key))
        };
        if is_duplicate {
            continue;
        }
        unique_keys.push(dedup_key);
        identities.push(candidate);
    }

    let resolved = stream::iter(identities.into_iter())
        .map(|address| {
            let state = state.clone();
            async move {
                resolve_remote_account_response_for_list(
                    state.config.as_ref(),
                    state.db.as_ref(),
                    state.profile_cache.as_ref(),
                    state.federation_fetch_client.as_ref(),
                    &address,
                    default_port,
                )
                .await
                .map(|response| serde_json::to_value(response).unwrap())
            }
        })
        .buffered(8)
        .collect::<Vec<_>>()
        .await;
    followers.extend(resolved.into_iter().flatten());

    let followers = paginate_account_response_values(followers, &params, limit);
    let first_id = followers
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = followers
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = account_collection_link_header(
        &format!("/api/v1/accounts/{id}/followers"),
        limit,
        first_id,
        last_id,
    ) {
        headers.insert(
            LINK,
            link.parse()
                .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
        );
    }

    Ok((headers, Json(followers)))
}

/// GET /api/v1/accounts/:id/following
pub async fn get_account_following(
    State(state): State<AccountApiState>,
    Path(id): Path<String>,
    Query(params): Query<PaginationParams>,
) -> Result<impl IntoResponse, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let limit = params.limit.unwrap_or(40).min(80);
    let local_requested = local_account_matches_identity(&account, state.config.as_ref(), &id);
    if !local_requested {
        let target_address = resolve_target_address(&state, &id).await?;
        let following =
            resolve_remote_collection_accounts(&state, &target_address, &params, "following")
                .await?;
        let first_id = following
            .first()
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str());
        let last_id = following
            .last()
            .and_then(|value| value.get("id"))
            .and_then(|value| value.as_str());
        let mut headers = HeaderMap::new();
        if let Some(link) = account_collection_link_header(
            &format!("/api/v1/accounts/{id}/following"),
            limit,
            first_id,
            last_id,
        ) {
            headers.insert(
                LINK,
                link.parse()
                    .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
            );
        }
        return Ok((headers, Json(following)));
    }

    let following_entries = state.db.get_all_follows().await?;
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let mut unique_keys: Vec<String> = Vec::new();
    let mut identities: Vec<String> = Vec::new();
    let mut following = Vec::new();

    for follow in following_entries {
        let Some(candidate) =
            list_entry_identity(follow.actor_uri.as_deref(), &follow.target_address)
        else {
            continue;
        };
        let Some(dedup_key) = list_entry_dedup_key(&candidate) else {
            continue;
        };

        let is_duplicate = if normalize_account_address(&dedup_key).is_ok() {
            contains_equivalent_address(&unique_keys, &dedup_key, default_port)
        } else {
            unique_keys
                .iter()
                .any(|seen| seen.eq_ignore_ascii_case(&dedup_key))
        };
        if is_duplicate {
            continue;
        }
        unique_keys.push(dedup_key);
        identities.push(candidate);
    }

    let resolved = stream::iter(identities.into_iter())
        .map(|address| {
            let state = state.clone();
            async move {
                resolve_remote_account_response_for_list(
                    state.config.as_ref(),
                    state.db.as_ref(),
                    state.profile_cache.as_ref(),
                    state.federation_fetch_client.as_ref(),
                    &address,
                    default_port,
                )
                .await
                .map(|response| serde_json::to_value(response).unwrap())
            }
        })
        .buffered(8)
        .collect::<Vec<_>>()
        .await;
    following.extend(resolved.into_iter().flatten());

    let following = paginate_account_response_values(following, &params, limit);
    let first_id = following
        .first()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let last_id = following
        .last()
        .and_then(|value| value.get("id"))
        .and_then(|value| value.as_str());
    let mut headers = HeaderMap::new();
    if let Some(link) = account_collection_link_header(
        &format!("/api/v1/accounts/{id}/following"),
        limit,
        first_id,
        last_id,
    ) {
        headers.insert(
            LINK,
            link.parse()
                .map_err(|_| AppError::Validation("invalid Link header".to_string()))?,
        );
    }

    Ok((headers, Json(following)))
}

/// POST /api/v1/accounts/:id/follow
pub async fn follow_account(
    State(state): State<AccountApiState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<String>,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, Follow};
    use chrono::Utc;

    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Get our account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_address = normalize_account_address(&format!(
        "{}@{}",
        account.username, state.config.server.domain
    ))?;
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let follow_request = if body.is_empty() {
        FollowAccountRequest::default()
    } else {
        parse_json_or_form_body::<FollowAccountRequest>(&parts.headers, &body)?
    };
    let follow_preferences = FollowPreferences {
        reblogs: follow_request.reblogs.unwrap_or(true),
        notify: follow_request.notify.unwrap_or(false),
    };

    if is_same_local_account(
        &target_address,
        &local_address,
        &state.config.server.protocol,
    ) {
        return Err(AppError::Validation("cannot follow yourself".to_string()));
    }

    let remote_follow_state = remote_profile_lock_state(&state, &target_address).await?;
    let target_actor_uri_hint = remote_follow_state
        .as_ref()
        .and_then(|(actor_uri, _)| actor_uri.clone());
    let target_is_locked = remote_follow_state
        .as_ref()
        .map(|(_, locked)| *locked)
        .unwrap_or(false);

    // Persist follow relationship if not already present.
    let follow_id = EntityId::new_string();
    let follow = Follow {
        id: follow_id.clone(),
        target_address: target_address.clone(),
        actor_uri: target_actor_uri_hint.clone(),
        uri: format!(
            "{}/users/{}/follow/{}",
            state.config.server.base_url(),
            account.username,
            follow_id
        ),
        created_at: Utc::now(),
    };
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let inserted = state
        .db
        .insert_follow_if_absent(&follow, default_port)
        .await?;
    save_follow_preferences(&state, &target_address, follow_preferences).await?;

    if !target_is_locked {
        let accepted_actor_uri = target_actor_uri_hint.as_deref().unwrap_or(&target_address);
        let _ = state
            .db
            .mark_follow_accepted(&target_address, accepted_actor_uri, default_port)
            .await?;
    }

    if inserted {
        let state_for_delivery = state.clone();
        let account_for_delivery = account.clone();
        let follow_uri = follow.uri.clone();
        let target_address_for_delivery = target_address.clone();
        spawn_best_effort_delivery("follow", async move {
            let (target_actor_uri, target_inbox_uri) =
                resolve_remote_actor_and_inbox(&state_for_delivery, &target_address_for_delivery)
                    .await?;
            if let Err(error) = state_for_delivery
                .db
                .update_follow_actor_uri(
                    &target_address_for_delivery,
                    &target_actor_uri,
                    default_port_for_protocol(&state_for_delivery.config.server.protocol),
                )
                .await
            {
                tracing::warn!(
                    "Failed to persist actor URI for follow target {}: {}",
                    target_address_for_delivery,
                    error
                );
            }
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .queue_follow_with_id(
                    state_for_delivery.db.as_ref(),
                    &follow_uri,
                    &target_actor_uri,
                    &target_inbox_uri,
                )
                .await
        });
    }

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

#[cfg(test)]
mod account_normalization_tests {
    use super::normalize_account_address;

    #[test]
    fn normalize_account_address_preserves_ipv6_brackets_without_port() {
        let normalized = normalize_account_address("Alice@[2001:DB8::1]").unwrap();
        assert_eq!(normalized, "alice@[2001:db8::1]");
    }

    #[test]
    fn normalize_account_address_preserves_ipv6_brackets_with_port() {
        let normalized = normalize_account_address("Alice@[2001:DB8::1]:443").unwrap();
        assert_eq!(normalized, "alice@[2001:db8::1]:443");
    }

    #[test]
    fn normalize_account_address_rejects_url_shaped_values() {
        assert!(normalize_account_address("https://remote.example/@alice").is_err());
    }
}

/// POST /api/v1/accounts/:id/unfollow
pub async fn unfollow_account(
    State(state): State<AccountApiState>,
    CurrentUser(_user): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Get our account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let follow = state.db.get_follow(&target_address, default_port).await?;
    let follow_uri = state
        .db
        .get_follow_uri(&target_address, default_port)
        .await?;

    // Remove follow relationship from DB.
    state
        .db
        .delete_follow(&target_address, default_port)
        .await?;

    if let Some(follow_uri) = follow_uri {
        let state_for_delivery = state.clone();
        let account_for_delivery = account.clone();
        let target_address_for_delivery = target_address.clone();
        let target_actor_uri_hint = follow.and_then(|follow| follow.actor_uri);
        spawn_best_effort_delivery("unfollow", async move {
            let (target_actor_uri, target_inbox_uri) = resolve_remote_actor_and_inbox_with_hint(
                &state_for_delivery,
                &target_address_for_delivery,
                target_actor_uri_hint.as_deref(),
            )
            .await?;
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .queue_undo_to_inbox_with_type_and_object(
                    state_for_delivery.db.as_ref(),
                    &follow_uri,
                    Some("Follow"),
                    Some(&target_actor_uri),
                    Some(&target_actor_uri),
                    &target_inbox_uri,
                )
                .await
        });
    }

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// GET /api/v1/accounts/relationships
pub async fn get_relationships(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    RawQuery(raw_query): RawQuery,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    use crate::api::dto::RelationshipResponse;

    let follows = state.db.get_all_follows().await?;
    let followers = state.db.get_all_followers().await?;
    let following_set: HashSet<String> = follows
        .iter()
        .filter_map(|follow| canonical_remote_account_address(&follow.target_address))
        .collect();
    let follower_set: HashSet<String> = followers
        .iter()
        .filter_map(|follower| canonical_remote_account_address(&follower.follower_address))
        .collect();
    let following_actor_uri_set: HashSet<String> = follows
        .iter()
        .filter_map(|follow| {
            follow
                .actor_uri
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect();
    let follower_actor_uri_set: HashSet<String> = followers
        .iter()
        .filter_map(|follower| {
            follower
                .actor_uri
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_string)
        })
        .collect();
    let default_port = default_port_for_protocol(&state.config.server.protocol);

    let ids: Vec<String> = raw_query
        .as_deref()
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .filter_map(|(key, value)| {
                    if key == "id[]" || key == "id" {
                        Some(value.into_owned())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();

    // Create relationship responses for each requested ID
    let mut relationships = vec![];
    for id in ids {
        let target_address = match resolve_target_address(&state, &id).await {
            Ok(address) => address,
            Err(_) => {
                let relationship = RelationshipResponse {
                    id: resolve_relationship_id(&state, &id).await,
                    following: false,
                    followed_by: false,
                    blocking: false,
                    blocked_by: false,
                    muting: false,
                    muting_notifications: false,
                    requested: false,
                    domain_blocking: false,
                    showing_reblogs: true,
                    endorsed: false,
                    notifying: false,
                    note: String::new(),
                };
                relationships.push(serde_json::to_value(relationship).unwrap());
                continue;
            }
        };
        let normalized_target = normalize_account_address(&target_address)
            .unwrap_or_else(|_| target_address.to_ascii_lowercase());
        let stored_following = following_actor_uri_set.contains(&id)
            || following_set.contains(&normalized_target)
            || following_set.iter().any(|candidate| {
                account_addresses_match_with_default_port(
                    candidate,
                    &normalized_target,
                    default_port,
                )
            });
        let followed_by = follower_actor_uri_set.contains(&id)
            || follower_set.contains(&normalized_target)
            || follower_set.iter().any(|candidate| {
                account_addresses_match_with_default_port(
                    candidate,
                    &normalized_target,
                    default_port,
                )
            });
        let blocking = state
            .db
            .is_account_blocked(&target_address, default_port)
            .await?;
        let muting = state
            .db
            .is_account_muted(&target_address, default_port)
            .await?;
        let muting_notifications = state
            .db
            .get_account_mute_notifications(&target_address, default_port)
            .await?
            .unwrap_or(false);
        let follow_is_accepted = state
            .db
            .is_follow_accepted(&target_address, default_port)
            .await?;
        let has_follow_request = state
            .db
            .has_follow_request_with_default_port(&target_address, default_port)
            .await?;
        let following = stored_following && follow_is_accepted;
        let requested = has_follow_request || (stored_following && !follow_is_accepted);
        let follow_preferences = load_follow_preferences(&state, &target_address).await?;

        let relationship = RelationshipResponse {
            id: resolve_relationship_id(&state, &id).await,
            following,
            followed_by,
            blocking,
            blocked_by: false,
            muting,
            muting_notifications,
            requested,
            domain_blocking: false,
            showing_reblogs: follow_preferences.reblogs,
            endorsed: false,
            notifying: follow_preferences.notify,
            note: String::new(),
        };

        relationships.push(serde_json::to_value(relationship).unwrap());
    }

    Ok(Json(relationships))
}

/// GET /api/v1/accounts/search
pub async fn search_accounts(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<SearchParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // For single-user instance, we can only search for:
    // 1. Our own account (by username)
    // 2. Remote accounts (by address like user@domain.com)

    let query = params.q.trim().to_lowercase();
    if query.is_empty() {
        return Ok(Json(Vec::new()));
    }
    let mut results = vec![];
    let local_domain = state.config.server.domain.to_ascii_lowercase();

    // Get our account
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let local_account_identity = canonical_account_identity(
        &format!("{}@{}", account.username, state.config.server.domain),
        &local_domain,
    );
    let query_identity = query
        .contains('@')
        .then(|| canonical_account_identity(&query, &local_domain));
    let mut matched_local_account = false;
    let limit = params.limit.unwrap_or(40).min(80);

    // Check if query matches our username
    if account.username.to_lowercase().contains(&query)
        || account
            .display_name
            .as_ref()
            .map(|d| d.to_lowercase().contains(&query))
            .unwrap_or(false)
        || query_identity
            .as_deref()
            .is_some_and(|identity| identity == local_account_identity)
    {
        let mut account_response = crate::api::account_to_response_with_stats(
            &account,
            &state.config,
            crate::api::AccountStats {
                followers_count: state.db.count_follower_addresses().await? as i32,
                following_count: state.db.count_follow_addresses().await? as i32,
                statuses_count: state.db.count_local_statuses().await? as i32,
            },
        );
        populate_local_account_compat_fields(&state, &account, &mut account_response).await;
        results.push(serde_json::to_value(account_response).unwrap());
        matched_local_account = true;
    }

    {
        let mut candidate_addresses = state
            .db
            .list_remote_profiles()
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|profile| {
                profile.address.to_ascii_lowercase().contains(&query)
                    || profile
                        .display_name
                        .as_ref()
                        .is_some_and(|value| value.to_ascii_lowercase().contains(&query))
            })
            .map(|profile| profile.address)
            .collect::<Vec<_>>();
        candidate_addresses.extend(
            state
                .db
                .get_all_follow_addresses()
                .await
                .unwrap_or_default(),
        );
        candidate_addresses.extend(
            state
                .db
                .get_all_follower_addresses()
                .await
                .unwrap_or_default(),
        );
        candidate_addresses.sort();
        candidate_addresses.dedup();

        for address in candidate_addresses {
            let address_matches = address.to_ascii_lowercase().contains(&query);
            let display_name_matches = state
                .profile_cache
                .get(&address)
                .await
                .and_then(|profile| profile.display_name.clone())
                .is_some_and(|value| value.to_ascii_lowercase().contains(&query));
            if !address_matches && !display_name_matches {
                continue;
            }

            if let Some(remote_account) = resolve_remote_account_response_for_list(
                state.config.as_ref(),
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                state.federation_fetch_client.as_ref(),
                &address,
                default_port_for_protocol(&state.config.server.protocol),
            )
            .await
            {
                let remote_identity =
                    canonical_account_identity(&remote_account.acct, &local_domain);
                let already_present = results.iter().any(|entry| {
                    entry
                        .get("acct")
                        .and_then(|value| value.as_str())
                        .is_some_and(|acct| {
                            canonical_account_identity(acct, &local_domain) == remote_identity
                        })
                });
                if !already_present {
                    results.push(serde_json::to_value(remote_account).unwrap());
                }
            }
        }
    }

    // If resolve=true and query looks like an account address, resolve and return profile info.
    if params.resolve.unwrap_or(false) && query.contains('@') {
        let should_skip_resolve = matched_local_account
            && query_identity.as_deref() == Some(local_account_identity.as_str());
        if !should_skip_resolve
            && let Some(remote_account) = resolve_remote_account_response(
                state.config.as_ref(),
                state.db.as_ref(),
                state.profile_cache.as_ref(),
                state.federation_fetch_client.as_ref(),
                &query,
            )
            .await
        {
            let remote_identity = canonical_account_identity(&remote_account.acct, &local_domain);
            let already_present = results.iter().any(|entry| {
                entry
                    .get("acct")
                    .and_then(|value| value.as_str())
                    .is_some_and(|acct| {
                        canonical_account_identity(acct, &local_domain) == remote_identity
                    })
            });
            if !already_present {
                results.push(serde_json::to_value(remote_account).unwrap());
            }
        }
    }

    if params.following.unwrap_or(false) {
        let following_addresses: HashSet<String> = state
            .db
            .get_all_follow_addresses()
            .await?
            .into_iter()
            .filter_map(|address| canonical_remote_account_address(&address))
            .collect();
        let following_actor_uris: HashSet<String> = state
            .db
            .get_all_follows()
            .await?
            .into_iter()
            .filter_map(|follow| follow.actor_uri)
            .collect();

        results.retain(|entry| {
            let acct_match = entry
                .get("acct")
                .and_then(|value| value.as_str())
                .map(|acct| canonical_account_identity(acct, &local_domain))
                .is_some_and(|acct| {
                    following_addresses.contains(&acct)
                        || following_addresses.iter().any(|candidate| {
                            account_addresses_match_with_default_port(
                                candidate,
                                &acct,
                                default_port_for_protocol(&state.config.server.protocol),
                            )
                        })
                });
            let url_match = entry
                .get("url")
                .and_then(|value| value.as_str())
                .is_some_and(|url| following_actor_uris.contains(url));

            acct_match || url_match
        });
    }

    let offset = params.offset.unwrap_or(0);
    results = results.into_iter().skip(offset).collect();
    results.truncate(limit);

    Ok(Json(results))
}

/// GET /api/v1/accounts/lookup
pub async fn lookup_account(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<LookupParams>,
) -> Result<Json<serde_json::Value>, AppError> {
    let identity = params.acct.trim();
    if identity.is_empty() {
        return Err(AppError::Validation("acct is required".to_string()));
    }

    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let response = if local_account_matches_identity(&account, state.config.as_ref(), identity) {
        let stats = crate::api::load_local_account_stats(state.db.as_ref()).await?;
        let mut response =
            crate::api::account_to_response_with_stats(&account, &state.config, stats);
        populate_local_account_compat_fields(&state, &account, &mut response).await;
        response
    } else if let Some(response) = resolve_cached_remote_account_response(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        identity,
    )
    .await
    {
        response
    } else if let Some(response) = resolve_remote_account_response(
        state.config.as_ref(),
        state.db.as_ref(),
        state.profile_cache.as_ref(),
        state.federation_fetch_client.as_ref(),
        identity,
    )
    .await
    {
        response
    } else {
        return Err(AppError::NotFound);
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// GET /api/v1/accounts/:id/lists
/// Get lists that contain the specified account
pub async fn get_account_lists(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let target_address = resolve_target_address(&state, &id).await?;
    let normalized_target = normalize_account_address(&target_address)?;
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let all_lists = state.db.get_all_lists().await?;
    let mut matched_lists = Vec::new();

    for (list_id, title, replies_policy, exclusive) in all_lists {
        let accounts = state.db.get_list_accounts(&list_id).await?;
        let contains_target = accounts.into_iter().any(|address| {
            address == id
                || account_addresses_match_with_default_port(
                    &address,
                    &normalized_target,
                    default_port,
                )
        });
        if contains_target {
            matched_lists.push(serde_json::json!({
                "id": list_id,
                "title": title,
                "replies_policy": replies_policy,
                "exclusive": exclusive,
            }));
        }
    }

    Ok(Json(matched_lists))
}

/// GET /api/v1/accounts/:id/identity_proofs
/// Get identity proofs for the specified account
///
/// Identity proofs (e.g., Keybase) are not supported,
/// so this always returns an empty array.
pub async fn get_account_identity_proofs(
    State(state): State<AccountApiState>,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let resolvable = local_account_matches_identity(&account, state.config.as_ref(), &id)
        || resolve_cached_remote_account_response(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            &id,
        )
        .await
        .is_some()
        || resolve_remote_account_response(
            state.config.as_ref(),
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            state.federation_fetch_client.as_ref(),
            &id,
        )
        .await
        .is_some();
    if !resolvable {
        return Err(AppError::NotFound);
    }

    Ok(Json(vec![]))
}

/// Mute account request
#[derive(Debug, Deserialize)]
pub struct MuteAccountRequest {
    pub notifications: Option<bool>,
    pub duration: Option<i64>, // Duration in seconds, 0 = indefinite
}

/// POST /api/v1/accounts/:id/block
/// Block an account
pub async fn block_account(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;
    let account_for_delivery = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let follow_actor_uri_hint = state
        .db
        .get_follow(&target_address, default_port)
        .await?
        .and_then(|follow| follow.actor_uri);
    let resolved_remote = resolve_remote_actor_and_inbox_with_hint(
        &state,
        &target_address,
        follow_actor_uri_hint.as_deref(),
    )
    .await
    .ok();
    let resolved_actor_uri = resolved_remote
        .as_ref()
        .map(|(actor_uri, _)| actor_uri.as_str());
    let resolved_inbox_uri = resolved_remote
        .as_ref()
        .map(|(_, inbox_uri)| inbox_uri.as_str());

    // Store block in database
    let newly_blocked = state
        .db
        .block_account_with_remote_metadata(
            &target_address,
            resolved_actor_uri,
            resolved_inbox_uri,
            default_port,
        )
        .await?;

    if newly_blocked {
        let state_for_delivery = state.clone();
        let account_for_delivery = account_for_delivery.clone();
        let target_address_for_delivery = target_address.clone();
        let resolved_remote_for_delivery = resolved_remote.clone();
        spawn_best_effort_delivery("block", async move {
            let (target_actor_uri, target_inbox_uri) =
                if let Some(resolved) = resolved_remote_for_delivery {
                    resolved
                } else {
                    resolve_remote_actor_and_inbox_with_hint(
                        &state_for_delivery,
                        &target_address_for_delivery,
                        follow_actor_uri_hint.as_deref(),
                    )
                    .await?
                };
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            delivery
                .queue_block_with_id(
                    state_for_delivery.db.as_ref(),
                    &delivery.block_activity_uri_for_target(&target_actor_uri),
                    &target_actor_uri,
                    &target_inbox_uri,
                )
                .await?;
            Ok(())
        });
    }

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/unblock
/// Unblock an account
pub async fn unblock_account(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;
    let account_for_delivery = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Remove block from database
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let block_target = state
        .db
        .get_block_target(&target_address, default_port)
        .await?;
    let unblocked = state
        .db
        .unblock_account(&target_address, default_port)
        .await?;

    if unblocked {
        let state_for_delivery = state.clone();
        let account_for_delivery = account_for_delivery.clone();
        let target_address_for_delivery = target_address.clone();
        let target_actor_uri_hint = block_target
            .as_ref()
            .and_then(|(_, actor_uri, _)| actor_uri.clone());
        let target_inbox_uri_hint = block_target
            .as_ref()
            .and_then(|(_, _, inbox_uri)| inbox_uri.clone());
        spawn_best_effort_delivery("unblock", async move {
            let (target_actor_uri, target_inbox_uri) =
                resolve_remote_actor_and_inbox_with_stored_hints(
                    &state_for_delivery,
                    &target_address_for_delivery,
                    target_actor_uri_hint.as_deref(),
                    target_inbox_uri_hint.as_deref(),
                )
                .await?;
            let delivery = build_delivery(&state_for_delivery, &account_for_delivery);
            let block_activity_uri = delivery.block_activity_uri_for_target(&target_actor_uri);
            delivery
                .queue_undo_to_inbox_with_type_and_object(
                    state_for_delivery.db.as_ref(),
                    &block_activity_uri,
                    Some("Block"),
                    Some(&target_actor_uri),
                    Some(&target_actor_uri),
                    &target_inbox_uri,
                )
                .await
        });
    }

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/mute
/// Mute an account
pub async fn mute_account(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    req: Option<Json<MuteAccountRequest>>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    let req = req
        .map(|Json(payload)| payload)
        .unwrap_or(MuteAccountRequest {
            notifications: None,
            duration: None,
        });

    let mute_notifications = req.notifications.unwrap_or(true);
    let duration = req.duration;
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let actor_uri_hint: Option<String> = if id.starts_with("http://") || id.starts_with("https://")
    {
        Some(id.clone())
    } else {
        state
            .db
            .get_follow(&target_address, default_port)
            .await?
            .and_then(|follow| follow.actor_uri)
    };

    // Store mute in database
    state
        .db
        .mute_account_with_actor_uri(
            &target_address,
            mute_notifications,
            duration,
            actor_uri_hint.as_deref(),
            default_port,
        )
        .await?;

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/accounts/:id/unmute
/// Unmute an account
pub async fn unmute_account(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Accept account addresses and local account IDs.
    let target_address = resolve_target_address(&state, &id).await?;

    // Remove mute from database
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    state
        .db
        .unmute_account(&target_address, default_port)
        .await?;

    let relationship = relationship_response_for_target(&state, &id, &target_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// GET /api/v1/blocks
/// Get list of blocked accounts
pub async fn get_blocks(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get blocked account addresses from database
    let limit = params.limit.unwrap_or(40).min(80);
    let default_port = default_port_for_protocol(&state.config.server.protocol);

    let blocked_accounts = state.db.get_blocked_account_details(limit).await?;
    let accounts = stream::iter(blocked_accounts.into_iter().take(limit))
        .map(|(address, actor_uri, _inbox_uri)| {
            let state = state.clone();
            async move {
                let identity = actor_uri.unwrap_or(address);
                resolve_remote_account_or_stub(state, identity, default_port).await
            }
        })
        .buffered(10)
        .collect::<Vec<_>>()
        .await;
    Ok(Json(accounts))
}

/// GET /api/v1/mutes
/// Get list of muted accounts
pub async fn get_mutes(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get muted account addresses from database
    let limit = params.limit.unwrap_or(40).min(80);
    let default_port = default_port_for_protocol(&state.config.server.protocol);

    let muted_accounts = state.db.get_muted_account_details(limit).await?;
    let accounts = stream::iter(muted_accounts.into_iter().take(limit))
        .map(|(address, actor_uri)| {
            let state = state.clone();
            async move {
                let identity = actor_uri.unwrap_or(address);
                resolve_remote_account_or_stub(state, identity, default_port).await
            }
        })
        .buffered(10)
        .collect::<Vec<_>>()
        .await;
    Ok(Json(accounts))
}

/// GET /api/v1/follow_requests
/// Get list of pending follow requests
pub async fn get_follow_requests(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Query(params): Query<PaginationParams>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    // Get follow requests from database
    let limit = params.limit.unwrap_or(40).min(80);
    let default_port = default_port_for_protocol(&state.config.server.protocol);

    let requests = state.db.get_follow_request_details(limit).await?;
    let accounts = stream::iter(requests.into_iter().take(limit))
        .map(|(address, actor_uri)| {
            let state = state.clone();
            async move {
                let identity = actor_uri.unwrap_or(address);
                resolve_remote_account_or_stub(state, identity, default_port).await
            }
        })
        .buffered(10)
        .collect::<Vec<_>>()
        .await;
    Ok(Json(accounts))
}

/// GET /api/v1/follow_requests/:id
/// Get a specific follow request
pub async fn get_follow_request(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let requester_address = resolve_follow_request_requester_address(&state, &id).await?;

    // Check if follow request exists
    let default_port = default_port_for_protocol(&state.config.server.protocol);
    let actor_identity = state
        .db
        .get_follow_request_with_actor_uri(&requester_address)
        .await?
        .and_then(|(_, _, actor_uri)| actor_uri)
        .unwrap_or_else(|| requester_address.clone());
    let account = resolve_remote_account_or_stub(state.clone(), actor_identity, default_port).await;

    Ok(Json(account))
}

/// POST /api/v1/follow_requests/:id/authorize
/// Accept a follow request
pub async fn authorize_follow_request(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let requester_address = resolve_follow_request_requester_address(&state, &id).await?;
    let (inbox_uri, follow_activity_uri) = state
        .db
        .get_follow_request(&requester_address)
        .await?
        .ok_or(AppError::NotFound)?;
    let account_for_delivery = state.db.get_account().await?.ok_or(AppError::NotFound)?;

    // Accept the follow request (moves to followers table)
    if !state.db.accept_follow_request(&requester_address).await? {
        return Err(AppError::NotFound);
    }

    let delivery = build_delivery(&state, &account_for_delivery);
    let db = state.db.clone();
    spawn_best_effort_delivery("authorize_follow_request", async move {
        delivery
            .queue_accept(db.as_ref(), &follow_activity_uri, &inbox_uri)
            .await
    });

    let relationship = relationship_response_for_target(&state, &id, &requester_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

/// POST /api/v1/follow_requests/:id/reject
/// Reject a follow request
pub async fn reject_follow_request(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let requester_address = resolve_follow_request_requester_address(&state, &id).await?;
    let follow_request = state.db.get_follow_request(&requester_address).await?;
    let account_for_delivery = if follow_request.is_some() {
        Some(state.db.get_account().await?.ok_or(AppError::NotFound)?)
    } else {
        None
    };

    // Remove from follow_requests
    if !state.db.reject_follow_request(&requester_address).await? {
        return Err(AppError::NotFound);
    }

    if let (Some((inbox_uri, follow_activity_uri)), Some(account_for_delivery)) =
        (follow_request, account_for_delivery)
    {
        let delivery = build_delivery(&state, &account_for_delivery);
        let db = state.db.clone();
        spawn_best_effort_delivery("reject_follow_request", async move {
            delivery
                .queue_reject(db.as_ref(), &follow_activity_uri, &inbox_uri)
                .await
        });
    }

    let relationship = relationship_response_for_target(&state, &id, &requester_address).await?;

    Ok(Json(serde_json::to_value(relationship).unwrap()))
}

#[cfg(test)]
mod image_decode_tests {
    use super::{decode_base64_image_field, normalize_image_bytes_to_webp};
    use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};

    const VALID_WEBP_BASE64: &str = "UklGRhoAAABXRUJQVlA4TA4AAAAvAAAAEM1VICIC0f+IBA==";
    const VALID_PNG_BASE64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4z8DwHwAFAAH/iZk9HQAAAABJRU5ErkJggg==";

    #[test]
    fn decode_base64_image_field_accepts_raw_base64() {
        let encoded = VALID_WEBP_BASE64;
        let decoded = decode_base64_image_field("avatar", encoded).expect("decode should succeed");
        assert_eq!(&decoded[0..4], b"RIFF");
        assert_eq!(&decoded[8..12], b"WEBP");
        assert!(decoded.len() >= 20);
    }

    #[test]
    fn decode_base64_image_field_accepts_data_url() {
        let encoded = format!("data:image/webp;base64,{}", VALID_WEBP_BASE64);
        let decoded = decode_base64_image_field("header", &encoded).expect("decode should succeed");
        assert_eq!(&decoded[0..4], b"RIFF");
        assert_eq!(&decoded[8..12], b"WEBP");
        assert!(decoded.len() >= 20);
    }

    #[test]
    fn decode_base64_image_field_rejects_non_base64_data_url() {
        let encoded = "data:image/webp,abc";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("base64"));
    }

    #[test]
    fn decode_base64_image_field_accepts_png_data_url() {
        let encoded = format!("data:image/png;base64,{}", VALID_PNG_BASE64);
        let decoded = decode_base64_image_field("avatar", &encoded).expect("decode should succeed");
        assert_eq!(&decoded[0..4], b"RIFF");
        assert_eq!(&decoded[8..12], b"WEBP");
        assert!(decoded.len() >= 20);
    }

    #[test]
    fn normalize_image_bytes_to_webp_accepts_png_bytes() {
        let decoded = normalize_image_bytes_to_webp(
            "avatar",
            BASE64_STANDARD.decode(VALID_PNG_BASE64).unwrap(),
        )
        .expect("decode should succeed");
        assert_eq!(&decoded[0..4], b"RIFF");
        assert_eq!(&decoded[8..12], b"WEBP");
        assert!(decoded.len() >= 20);
    }

    #[test]
    fn decode_base64_image_field_rejects_raw_non_webp_bytes() {
        let encoded = "aGVsbG8=";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_truncated_webp_header_only() {
        let encoded = "UklGRgAAAABXRUJQ";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_invalid_vp8x_chunk_payload() {
        let encoded = "UklGRgwAAABXRUJQVlA4WAAAAAA=";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_vp8x_header_without_frame_data() {
        let encoded = "UklGRhYAAABXRUJQVlA4WAoAAAAAAAAAAAAAAAAA";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_invalid_vp8_frame_header() {
        let encoded = "UklGRhYAAABXRUJQVlA4IAoAAAAAAACdASoAAAAA";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_invalid_vp8l_header_fields() {
        let encoded = "UklGRhIAAABXRUJQVlA4TAUAAAAvAAAA4AA=";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }

    #[test]
    fn decode_base64_image_field_rejects_non_decodable_vp8_payload() {
        let encoded = "UklGRhgAAABXRUJQVlA4IAwAAAAAAQCdASoEAA0AGsY=";
        let error = decode_base64_image_field("avatar", encoded).expect_err("must fail");
        assert!(format!("{error}").contains("supported image"));
    }
}
