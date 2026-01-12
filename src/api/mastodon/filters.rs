//! Filters endpoints

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::auth::CurrentUser;
use crate::error::AppError;

/// Filter response (v1 API)
#[derive(Debug, Serialize)]
pub struct FilterResponse {
    pub id: String,
    pub phrase: String,
    pub context: Vec<String>,
    pub expires_at: Option<String>,
    pub irreversible: bool,
    pub whole_word: bool,
}

/// Create filter request
#[derive(Debug, Deserialize)]
pub struct CreateFilterRequest {
    pub phrase: String,
    pub context: Vec<String>, // ["home", "notifications", "public", "thread"]
    pub expires_in: Option<i64>, // Seconds from now
    pub irreversible: Option<bool>,
    pub whole_word: Option<bool>,
}

/// Update filter request
#[derive(Debug, Deserialize)]
pub struct UpdateFilterRequest {
    pub phrase: Option<String>,
    pub context: Option<Vec<String>>,
    pub expires_in: Option<i64>,
    pub irreversible: Option<bool>,
    pub whole_word: Option<bool>,
}

/// GET /api/v1/filters
/// Get all filters
pub async fn get_filters(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<FilterResponse>>, AppError> {
    let filters = state.db.get_all_filters().await?;

    let response: Vec<FilterResponse> = filters
        .into_iter()
        .map(
            |(id, phrase, context, expires_at, irreversible, whole_word)| {
                // Parse context string (comma-separated) into Vec
                let context_vec: Vec<String> = context
                    .split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();

                FilterResponse {
                    id,
                    phrase,
                    context: context_vec,
                    expires_at,
                    irreversible,
                    whole_word,
                }
            },
        )
        .collect();

    Ok(Json(response))
}

/// GET /api/v1/filters/:id
/// Get a specific filter
pub async fn get_filter(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<FilterResponse>, AppError> {
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;

    // Parse context string into Vec
    let context_vec: Vec<String> = filter
        .2
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    Ok(Json(FilterResponse {
        id: filter.0,
        phrase: filter.1,
        context: context_vec,
        expires_at: filter.3,
        irreversible: filter.4,
        whole_word: filter.5,
    }))
}

/// POST /api/v1/filters
/// Create a new filter
pub async fn create_filter(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Json(req): Json<CreateFilterRequest>,
) -> Result<Json<FilterResponse>, AppError> {
    // Validate phrase
    if req.phrase.trim().is_empty() {
        return Err(AppError::Validation("Phrase cannot be empty".to_string()));
    }

    // Validate context
    if req.context.is_empty() {
        return Err(AppError::Validation(
            "At least one context is required".to_string(),
        ));
    }

    // Validate context values
    for ctx in &req.context {
        if !["home", "notifications", "public", "thread", "account"].contains(&ctx.as_str()) {
            return Err(AppError::Validation(format!(
                "Invalid context '{}'. Must be 'home', 'notifications', 'public', 'thread', or 'account'",
                ctx
            )));
        }
    }

    // Join context array into comma-separated string
    let context_str = req.context.join(",");

    // Calculate expires_at if expires_in is provided
    let expires_at = req.expires_in.map(|seconds| {
        let expires = chrono::Utc::now() + chrono::Duration::seconds(seconds);
        expires.to_rfc3339()
    });

    let irreversible = req.irreversible.unwrap_or(false);
    let whole_word = req.whole_word.unwrap_or(true);

    let id = state
        .db
        .create_filter(
            &req.phrase,
            &context_str,
            expires_at.as_deref(),
            irreversible,
            whole_word,
        )
        .await?;

    Ok(Json(FilterResponse {
        id,
        phrase: req.phrase,
        context: req.context,
        expires_at,
        irreversible,
        whole_word,
    }))
}

/// PUT /api/v1/filters/:id
/// Update a filter
pub async fn update_filter(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateFilterRequest>,
) -> Result<Json<FilterResponse>, AppError> {
    // Get existing filter
    let existing = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;

    // Use existing values if not provided
    let phrase = req.phrase.unwrap_or(existing.1.clone());
    let context_vec = if let Some(ctx) = req.context {
        // Validate new context
        for c in &ctx {
            if !["home", "notifications", "public", "thread", "account"].contains(&c.as_str()) {
                return Err(AppError::Validation(format!(
                    "Invalid context '{}'. Must be 'home', 'notifications', 'public', 'thread', or 'account'",
                    c
                )));
            }
        }
        ctx
    } else {
        // Parse existing context
        existing
            .2
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect()
    };

    let context_str = context_vec.join(",");

    // Calculate new expires_at if expires_in is provided
    let expires_at = if let Some(seconds) = req.expires_in {
        let expires = chrono::Utc::now() + chrono::Duration::seconds(seconds);
        Some(expires.to_rfc3339())
    } else {
        existing.3.clone()
    };

    let irreversible = req.irreversible.unwrap_or(existing.4);
    let whole_word = req.whole_word.unwrap_or(existing.5);

    // Validate phrase
    if phrase.trim().is_empty() {
        return Err(AppError::Validation("Phrase cannot be empty".to_string()));
    }

    state
        .db
        .update_filter(
            &id,
            &phrase,
            &context_str,
            expires_at.as_deref(),
            irreversible,
            whole_word,
        )
        .await?;

    Ok(Json(FilterResponse {
        id,
        phrase,
        context: context_vec,
        expires_at,
        irreversible,
        whole_word,
    }))
}

/// DELETE /api/v1/filters/:id
/// Delete a filter
pub async fn delete_filter(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_filter(&id).await?;

    if !deleted {
        return Err(AppError::NotFound);
    }

    Ok(Json(serde_json::json!({})))
}

/// GET /api/v2/filters
/// Get all filters (v2 API)
///
/// For now, this returns the same as v1 API
/// In the future, this should return filters with keywords
pub async fn get_filters_v2(
    State(state): State<AppState>,
    CurrentUser(session): CurrentUser,
) -> Result<Json<Vec<FilterResponse>>, AppError> {
    // For now, just return v1 filters
    get_filters(State(state), CurrentUser(session)).await
}
