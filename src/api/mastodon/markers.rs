use axum::{extract::State, response::Json};
use serde::{Deserialize, Serialize};

use crate::TimelineApiState;
use crate::auth::CurrentUser;
use crate::error::AppError;

const HOME_MARKER_KEY: &str = "markers.home.last_read_id";
const NOTIFICATIONS_MARKER_KEY: &str = "markers.notifications.last_read_id";

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
    Ok(Some(Marker {
        last_read_id: value,
        version: 1,
        updated_at: chrono::Utc::now().to_rfc3339(),
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
    Ok(Marker {
        last_read_id: last_read_id.to_string(),
        version: 1,
        updated_at: chrono::Utc::now().to_rfc3339(),
    })
}

pub async fn get_markers(
    State(state): State<TimelineApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<serde_json::Value>, AppError> {
    let envelope = MarkerEnvelope {
        home: load_marker(&state, HOME_MARKER_KEY).await?,
        notifications: load_marker(&state, NOTIFICATIONS_MARKER_KEY).await?,
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
