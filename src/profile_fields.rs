use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::{data::ProfileField, error::AppError};

pub(crate) const MAX_PROFILE_FIELDS: usize = 4;
const MAX_PROFILE_FIELD_NAME_LEN: usize = 255;
const MAX_PROFILE_FIELD_VALUE_LEN: usize = 255;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ProfileFieldInput {
    name: String,
    value: String,
}

pub(crate) fn normalize_profile_fields_request(
    fields_attributes: Option<&serde_json::Value>,
) -> Result<Option<Option<String>>, AppError> {
    let Some(fields_attributes) = fields_attributes else {
        return Ok(None);
    };

    let inputs = match fields_attributes {
        serde_json::Value::Null => Vec::new(),
        serde_json::Value::Array(items) => items
            .iter()
            .filter(|value| value.is_object())
            .map(|value| serde_json::from_value::<ProfileFieldInput>(value.clone()))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| AppError::Validation(format!("invalid fields_attributes: {error}")))?,
        serde_json::Value::Object(map) => {
            let mut entries = map
                .iter()
                .filter_map(|(index, value)| value.as_object().map(|_| (index, value.clone())))
                .collect::<Vec<_>>();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            entries
                .into_iter()
                .map(|(_, value)| serde_json::from_value::<ProfileFieldInput>(value))
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| AppError::Validation(format!("invalid fields_attributes: {error}")))?
        }
        _ => {
            return Err(AppError::Validation(
                "fields_attributes must be an object, array, or null".to_string(),
            ));
        }
    };

    if inputs.len() > MAX_PROFILE_FIELDS {
        return Err(AppError::Validation(format!(
            "fields_attributes must contain at most {MAX_PROFILE_FIELDS} entries"
        )));
    }

    let fields = inputs
        .into_iter()
        .filter_map(|field| {
            let name = field.name.trim().to_string();
            let value = field.value.trim().to_string();
            if name.is_empty() && value.is_empty() {
                return None;
            }
            Some((name, value))
        })
        .map(|(name, value)| {
            if name.len() > MAX_PROFILE_FIELD_NAME_LEN {
                return Err(AppError::Validation(format!(
                    "profile field names must be at most {MAX_PROFILE_FIELD_NAME_LEN} characters"
                )));
            }
            if value.len() > MAX_PROFILE_FIELD_VALUE_LEN {
                return Err(AppError::Validation(format!(
                    "profile field values must be at most {MAX_PROFILE_FIELD_VALUE_LEN} characters"
                )));
            }
            Ok(ProfileField {
                name,
                value,
                verified_at: None,
            })
        })
        .collect::<Result<Vec<_>, _>>()?;

    if fields.is_empty() {
        return Ok(Some(None));
    }

    let json = serde_json::to_string(&fields)
        .map_err(|error| AppError::internal(format!("serialize profile fields: {error}")))?;
    Ok(Some(Some(json)))
}

pub(crate) fn parse_profile_fields_json(raw: Option<&str>) -> Vec<ProfileField> {
    raw.and_then(|value| serde_json::from_str::<Vec<ProfileField>>(value).ok())
        .unwrap_or_default()
}

pub(crate) fn serialize_profile_fields(fields: &[ProfileField]) -> Result<Option<String>, AppError> {
    if fields.is_empty() {
        return Ok(None);
    }
    serde_json::to_string(fields)
        .map(Some)
        .map_err(|error| AppError::internal(format!("serialize profile fields: {error}")))
}

pub(crate) fn profile_fields_for_response(raw: Option<&str>) -> Vec<serde_json::Value> {
    parse_profile_fields_json(raw)
        .into_iter()
        .map(|field| {
            serde_json::json!({
                "name": field.name,
                "value": render_profile_field_value(&field.value),
                "verified_at": field.verified_at.map(|value| value.to_rfc3339()),
            })
        })
        .collect()
}

pub(crate) fn profile_fields_for_source(raw: Option<&str>) -> Vec<serde_json::Value> {
    parse_profile_fields_json(raw)
        .into_iter()
        .map(|field| {
            serde_json::json!({
                "name": field.name,
                "value": field.value,
                "verified_at": field.verified_at.map(|value| value.to_rfc3339()),
            })
        })
        .collect()
}

pub(crate) fn activitypub_profile_attachments(raw: Option<&str>) -> Vec<serde_json::Value> {
    parse_profile_fields_json(raw)
        .into_iter()
        .map(|field| {
            serde_json::json!({
                "type": "PropertyValue",
                "name": field.name,
                "value": render_profile_field_value(&field.value),
            })
        })
        .collect()
}

pub(crate) fn extract_profile_fields_from_actor(actor_document: &serde_json::Value) -> Vec<ProfileField> {
    actor_document
        .get("attachment")
        .and_then(serde_json::Value::as_array)
        .into_iter()
        .flat_map(|items| items.iter())
        .filter_map(|item| {
            let name = item.get("name")?.as_str()?.trim();
            let value = item.get("value")?.as_str()?.trim();
            if name.is_empty() && value.is_empty() {
                return None;
            }
            Some(ProfileField {
                name: name.to_string(),
                value: value.to_string(),
                verified_at: item
                    .get("verified_at")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                    .map(|value| value.with_timezone(&Utc)),
            })
        })
        .collect()
}

fn render_profile_field_value(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    if trimmed.contains('<') && trimmed.contains('>') {
        return trimmed.to_string();
    }

    if let Ok(parsed) = url::Url::parse(trimmed)
        && matches!(parsed.scheme(), "http" | "https")
    {
        let escaped = html_escape::encode_text(trimmed);
        return format!(
            "<a href=\"{escaped}\" rel=\"me nofollow noopener noreferrer\" target=\"_blank\">{escaped}</a>"
        );
    }

    html_escape::encode_text(trimmed)
        .replace('\n', "<br />")
        .to_string()
}
