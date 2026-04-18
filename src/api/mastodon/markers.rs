use axum::{
    body::to_bytes,
    extract::{RawQuery, Request, State},
    http::{HeaderMap, header::CONTENT_TYPE},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::error::AppError;

const HOME_MARKER_KEY: &str = "markers.home.last_read_id";
const NOTIFICATIONS_MARKER_KEY: &str = "markers.notifications.last_read_id";
const HOME_MARKER_UPDATED_AT_KEY: &str = "markers.home.updated_at";
const NOTIFICATIONS_MARKER_UPDATED_AT_KEY: &str = "markers.notifications.updated_at";
const HOME_MARKER_VERSION_KEY: &str = "markers.home.version";
const NOTIFICATIONS_MARKER_VERSION_KEY: &str = "markers.notifications.version";

#[derive(Debug, Default, Deserialize)]
pub struct SaveMarkersRequest {
    pub home: Option<MarkerUpdateRequest>,
    pub notifications: Option<MarkerUpdateRequest>,
}

#[derive(Debug, Deserialize)]
pub struct MarkerUpdateRequest {
    pub last_read_id: String,
}

fn parse_markers_form(body: &[u8]) -> SaveMarkersRequest {
    let mut request = SaveMarkersRequest::default();
    for (key, value) in url::form_urlencoded::parse(body).into_owned() {
        match key.as_str() {
            "home[last_read_id]" => {
                request.home = Some(MarkerUpdateRequest {
                    last_read_id: value,
                });
            }
            "notifications[last_read_id]" => {
                request.notifications = Some(MarkerUpdateRequest {
                    last_read_id: value,
                });
            }
            _ => {}
        }
    }
    request
}

fn parse_save_markers_request(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<SaveMarkersRequest, AppError> {
    let content_type = headers
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if content_type.starts_with("application/x-www-form-urlencoded") {
        return Ok(parse_markers_form(body));
    }
    if body.is_empty() {
        return Ok(SaveMarkersRequest::default());
    }
    serde_json::from_slice(body)
        .map_err(|error| AppError::Validation(format!("invalid JSON body: {error}")))
}

#[derive(Debug, Serialize)]
pub struct MarkerEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub home: Option<Marker>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notifications: Option<Marker>,
}

#[derive(Debug, Serialize)]
pub struct Marker {
    pub last_read_id: String,
    pub version: i32,
    pub updated_at: String,
}

async fn load_marker(state: &TimelineApiState, key: &str) -> Result<Option<Marker>, AppError> {
    let Some(value) = state.db.get_setting(key).await? else {
        return Ok(None);
    };
    let (updated_at_key, version_key) = match key {
        HOME_MARKER_KEY => (HOME_MARKER_UPDATED_AT_KEY, HOME_MARKER_VERSION_KEY),
        NOTIFICATIONS_MARKER_KEY => (
            NOTIFICATIONS_MARKER_UPDATED_AT_KEY,
            NOTIFICATIONS_MARKER_VERSION_KEY,
        ),
        _ => unreachable!("unknown marker key"),
    };
    let updated_at = state
        .db
        .get_setting(updated_at_key)
        .await?
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    Ok(Some(Marker {
        last_read_id: value,
        version: state
            .db
            .get_setting(version_key)
            .await?
            .and_then(|value| value.trim().parse::<i32>().ok())
            .filter(|value| *value > 0)
            .unwrap_or(1),
        updated_at,
    }))
}

async fn save_marker(
    state: &TimelineApiState,
    key: &str,
    value: MarkerUpdateRequest,
) -> Result<Marker, AppError> {
    let last_read_id = value.last_read_id.trim();
    if last_read_id.is_empty() {
        return Err(AppError::Validation(
            "last_read_id must not be empty".to_string(),
        ));
    }

    state.db.set_setting(key, last_read_id).await?;
    let updated_at = chrono::Utc::now().to_rfc3339();
    let (updated_at_key, version_key) = match key {
        HOME_MARKER_KEY => (HOME_MARKER_UPDATED_AT_KEY, HOME_MARKER_VERSION_KEY),
        NOTIFICATIONS_MARKER_KEY => (
            NOTIFICATIONS_MARKER_UPDATED_AT_KEY,
            NOTIFICATIONS_MARKER_VERSION_KEY,
        ),
        _ => unreachable!("unknown marker key"),
    };
    let version = state
        .db
        .get_setting(version_key)
        .await?
        .and_then(|value| value.trim().parse::<i32>().ok())
        .filter(|value| *value >= 0)
        .unwrap_or(0)
        .saturating_add(1);
    state.db.set_setting(updated_at_key, &updated_at).await?;
    state
        .db
        .set_setting(version_key, &version.to_string())
        .await?;
    Ok(Marker {
        last_read_id: last_read_id.to_string(),
        version,
        updated_at,
    })
}

pub async fn get_markers(
    State(state): State<TimelineApiState>,
    raw_query: RawQuery,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let explicit_timelines = raw_query
        .0
        .as_deref()
        .map(|query| {
            url::form_urlencoded::parse(query.as_bytes())
                .filter_map(|(key, value)| {
                    ((key == "timeline[]" || key == "timeline") && !value.trim().is_empty())
                        .then(|| value.into_owned())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
        .into_iter()
        .map(|timeline| timeline.trim().to_string())
        .filter(|timeline| !timeline.is_empty())
        .collect::<Vec<_>>();
    if explicit_timelines.is_empty() {
        return Ok(Json(serde_json::json!({})));
    }

    let wants_home = explicit_timelines
        .iter()
        .any(|timeline| *timeline == "home");
    let wants_notifications = explicit_timelines
        .iter()
        .any(|timeline| *timeline == "notifications");
    let envelope = MarkerEnvelope {
        home: if wants_home {
            load_marker(&state, HOME_MARKER_KEY).await?
        } else {
            None
        },
        notifications: if wants_notifications {
            load_marker(&state, NOTIFICATIONS_MARKER_KEY).await?
        } else {
            None
        },
    };
    Ok(Json(serde_json::to_value(envelope).unwrap()))
}

pub async fn save_markers(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
    request: Request,
) -> Result<Json<serde_json::Value>, AppError> {
    let (parts, body) = request.into_parts();
    let body = to_bytes(body, 64 * 1024)
        .await
        .map_err(|error| AppError::Validation(format!("failed to read request body: {error}")))?;
    let request = parse_save_markers_request(&parts.headers, &body)?;
    let mut response = serde_json::Map::new();
    if let Some(home) = request.home {
        response.insert(
            "home".to_string(),
            serde_json::to_value(save_marker(&state, HOME_MARKER_KEY, home).await?).unwrap(),
        );
    }
    if let Some(notifications) = request.notifications {
        response.insert(
            "notifications".to_string(),
            serde_json::to_value(
                save_marker(&state, NOTIFICATIONS_MARKER_KEY, notifications).await?,
            )
            .unwrap(),
        );
    }
    Ok(Json(serde_json::Value::Object(response)))
}
