use axum::{
    extract::{Query, State},
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

#[derive(Debug, Default, Deserialize)]
pub struct GetMarkersParams {
    #[serde(rename = "timeline[]", default)]
    pub timelines: Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SaveMarkersRequest {
    pub home: Option<MarkerUpdateRequest>,
    pub notifications: Option<MarkerUpdateRequest>,
}

#[derive(Debug, Deserialize)]
pub struct MarkerUpdateRequest {
    pub last_read_id: String,
}

#[derive(Debug, Serialize)]
pub struct MarkerEnvelope {
    pub home: Option<Marker>,
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
    let updated_at_key = match key {
        HOME_MARKER_KEY => HOME_MARKER_UPDATED_AT_KEY,
        NOTIFICATIONS_MARKER_KEY => NOTIFICATIONS_MARKER_UPDATED_AT_KEY,
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
        version: 1,
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
    let updated_at_key = match key {
        HOME_MARKER_KEY => HOME_MARKER_UPDATED_AT_KEY,
        NOTIFICATIONS_MARKER_KEY => NOTIFICATIONS_MARKER_UPDATED_AT_KEY,
        _ => unreachable!("unknown marker key"),
    };
    state.db.set_setting(updated_at_key, &updated_at).await?;
    Ok(Marker {
        last_read_id: last_read_id.to_string(),
        version: 1,
        updated_at,
    })
}

pub async fn get_markers(
    State(state): State<TimelineApiState>,
    Query(params): Query<GetMarkersParams>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let wants_home =
        params.timelines.is_empty() || params.timelines.iter().any(|timeline| timeline == "home");
    let wants_notifications = params.timelines.is_empty()
        || params
            .timelines
            .iter()
            .any(|timeline| timeline == "notifications");
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
    Json(request): Json<SaveMarkersRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let envelope = MarkerEnvelope {
        home: match request.home {
            Some(home) => Some(save_marker(&state, HOME_MARKER_KEY, home).await?),
            None => load_marker(&state, HOME_MARKER_KEY).await?,
        },
        notifications: match request.notifications {
            Some(notifications) => {
                Some(save_marker(&state, NOTIFICATIONS_MARKER_KEY, notifications).await?)
            }
            None => load_marker(&state, NOTIFICATIONS_MARKER_KEY).await?,
        },
    };
    Ok(Json(serde_json::to_value(envelope).unwrap()))
}
