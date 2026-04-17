//! Filters endpoints

use axum::{
    extract::{Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::FiltersApiState;
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

#[derive(Debug, Deserialize)]
pub struct CreateFilterV2KeywordRequest {
    pub keyword: String,
    pub whole_word: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct CreateFilterV2Request {
    pub title: Option<String>,
    pub context: Vec<String>,
    pub expires_in: Option<i64>,
    pub filter_action: Option<String>,
    pub keywords: Option<Vec<CreateFilterV2KeywordRequest>>,
    pub phrase: Option<String>,
    pub whole_word: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateFilterV2Request {
    pub title: Option<String>,
    pub context: Option<Vec<String>>,
    pub expires_in: Option<i64>,
    pub filter_action: Option<String>,
    pub keywords: Option<Vec<CreateFilterV2KeywordRequest>>,
    pub whole_word: Option<bool>,
}

fn filter_v2_value(
    id: &str,
    title: &str,
    context: &[String],
    expires_at: Option<String>,
    irreversible: bool,
    keywords: Vec<serde_json::Value>,
    statuses: Vec<serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "title": title,
        "context": context,
        "expires_at": expires_at,
        "filter_action": if irreversible { "hide" } else { "warn" },
        "keywords": keywords,
        "statuses": statuses
    })
}

fn validate_filter_context(context: &[String]) -> Result<(), AppError> {
    if context.is_empty() {
        return Err(AppError::Validation(
            "At least one context is required".to_string(),
        ));
    }
    for ctx in context {
        if !["home", "notifications", "public", "thread", "account"].contains(&ctx.as_str()) {
            return Err(AppError::Validation(format!(
                "Invalid context '{}'. Must be 'home', 'notifications', 'public', 'thread', or 'account'",
                ctx
            )));
        }
    }
    Ok(())
}

fn filter_keyword_value(id: &str, phrase: &str, whole_word: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "keyword": phrase,
        "whole_word": whole_word,
    })
}

fn parse_filter_context(context: &str) -> Vec<String> {
    context
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

async fn load_filter_keywords_value(
    state: &FiltersApiState,
    filter_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    let keywords = state.db.get_filter_keywords(filter_id).await?;
    Ok(keywords
        .into_iter()
        .map(|(keyword_id, keyword, whole_word)| {
            filter_keyword_value(&keyword_id, &keyword, whole_word)
        })
        .collect())
}

async fn load_filter_statuses_value(
    state: &FiltersApiState,
    filter_id: &str,
) -> Result<Vec<serde_json::Value>, AppError> {
    Ok(state
        .db
        .get_filter_statuses(filter_id)
        .await?
        .into_iter()
        .map(|(id, status_id)| {
            serde_json::json!({
                "id": id,
                "status_id": status_id,
            })
        })
        .collect())
}

async fn filter_v2_value_from_row(
    state: &FiltersApiState,
    filter: (String, String, String, Option<String>, bool, bool),
) -> Result<serde_json::Value, AppError> {
    let context = parse_filter_context(&filter.2);
    let keywords = load_filter_keywords_value(state, &filter.0).await?;
    let statuses = load_filter_statuses_value(state, &filter.0).await?;
    Ok(filter_v2_value(
        &filter.0, &filter.1, &context, filter.3, filter.4, keywords, statuses,
    ))
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
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<FilterResponse>>, AppError> {
    let filters = state.db.get_all_filters().await?;

    let response: Vec<FilterResponse> = filters
        .into_iter()
        .map(
            |(id, phrase, context, expires_at, irreversible, whole_word)| FilterResponse {
                id,
                phrase,
                context: parse_filter_context(&context),
                expires_at,
                irreversible,
                whole_word,
            },
        )
        .collect();

    Ok(Json(response))
}

/// GET /api/v1/filters/:id
/// Get a specific filter
pub async fn get_filter(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<FilterResponse>, AppError> {
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;

    // Parse context string into Vec
    let context_vec = parse_filter_context(&filter.2);

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
    State(state): State<FiltersApiState>,
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
    state
        .db
        .replace_filter_keywords(&id, &[(req.phrase.clone(), whole_word)])
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
    State(state): State<FiltersApiState>,
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
        parse_filter_context(&existing.2)
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
    state
        .db
        .replace_filter_keywords(&id, &[(phrase.clone(), whole_word)])
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
    State(state): State<FiltersApiState>,
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
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let filters = state.db.get_all_filters().await?;
    let mut response = Vec::with_capacity(filters.len());
    for filter in filters {
        response.push(filter_v2_value_from_row(&state, filter).await?);
    }
    Ok(Json(response))
}

pub async fn get_filter_v2(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(filter_v2_value_from_row(&state, filter).await?))
}

pub async fn create_filter_v2(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Json(request): Json<CreateFilterV2Request>,
) -> Result<Json<serde_json::Value>, AppError> {
    validate_filter_context(&request.context)?;
    let title = request
        .title
        .clone()
        .or_else(|| request.phrase.clone())
        .or_else(|| {
            request
                .keywords
                .as_ref()
                .and_then(|keywords| keywords.first().map(|keyword| keyword.keyword.clone()))
        })
        .ok_or_else(|| {
            AppError::Validation("title, phrase, or keywords is required".to_string())
        })?;
    let keywords = if let Some(keywords) = request.keywords.as_ref() {
        if keywords.is_empty() {
            return Err(AppError::Validation(
                "keywords must not be empty when provided".to_string(),
            ));
        }
        keywords
            .iter()
            .map(|keyword| {
                let value = keyword.keyword.trim();
                if value.is_empty() {
                    return Err(AppError::Validation("keyword cannot be empty".to_string()));
                }
                Ok((value.to_string(), keyword.whole_word.unwrap_or(true)))
            })
            .collect::<Result<Vec<_>, AppError>>()?
    } else {
        vec![(title.clone(), request.whole_word.unwrap_or(true))]
    };
    let context_str = request.context.join(",");
    let expires_at = request.expires_in.map(|seconds| {
        let expires = chrono::Utc::now() + chrono::Duration::seconds(seconds);
        expires.to_rfc3339()
    });
    let irreversible = match request.filter_action.as_deref().unwrap_or("warn") {
        "warn" => false,
        "hide" => true,
        _ => {
            return Err(AppError::Validation(
                "filter_action must be 'warn' or 'hide'".to_string(),
            ));
        }
    };
    let whole_word = keywords.first().map(|(_, value)| *value).unwrap_or(true);
    let id = state
        .db
        .create_filter(
            &title,
            &context_str,
            expires_at.as_deref(),
            irreversible,
            whole_word,
        )
        .await?;
    state.db.replace_filter_keywords(&id, &keywords).await?;
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(filter_v2_value_from_row(&state, filter).await?))
}

pub async fn update_filter_v2(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(request): Json<UpdateFilterV2Request>,
) -> Result<Json<serde_json::Value>, AppError> {
    let existing = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    if let Some(context) = request.context.as_ref() {
        validate_filter_context(context)?;
    }
    let phrase = request.title.unwrap_or(existing.1.clone());
    let context = request
        .context
        .unwrap_or_else(|| parse_filter_context(&existing.2));
    let context_str = context.join(",");
    let expires_at = if let Some(seconds) = request.expires_in {
        Some((chrono::Utc::now() + chrono::Duration::seconds(seconds)).to_rfc3339())
    } else {
        existing.3.clone()
    };
    let irreversible = request
        .filter_action
        .as_deref()
        .map(|action| match action {
            "warn" => Ok(false),
            "hide" => Ok(true),
            _ => Err(AppError::Validation(
                "filter_action must be 'warn' or 'hide'".to_string(),
            )),
        })
        .transpose()?
        .unwrap_or(existing.4);
    let keywords = if let Some(keywords) = request.keywords.as_ref() {
        if keywords.is_empty() {
            return Err(AppError::Validation(
                "keywords must not be empty when provided".to_string(),
            ));
        }
        keywords
            .iter()
            .map(|keyword| {
                let value = keyword.keyword.trim();
                if value.is_empty() {
                    return Err(AppError::Validation("keyword cannot be empty".to_string()));
                }
                Ok((value.to_string(), keyword.whole_word.unwrap_or(true)))
            })
            .collect::<Result<Vec<_>, AppError>>()?
    } else {
        state
            .db
            .get_filter_keywords(&id)
            .await?
            .into_iter()
            .map(|(_, keyword, whole_word)| (keyword, whole_word))
            .collect::<Vec<_>>()
    };
    let whole_word = keywords
        .first()
        .map(|(_, whole_word)| *whole_word)
        .or(request.whole_word)
        .unwrap_or(existing.5);
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
    if !keywords.is_empty() {
        state.db.replace_filter_keywords(&id, &keywords).await?;
    }
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(filter_v2_value_from_row(&state, filter).await?))
}

pub async fn get_filter_keywords(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let filter = state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(load_filter_keywords_value(&state, &filter.0).await?))
}

pub async fn create_filter_keyword(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(request): Json<CreateFilterV2KeywordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    let phrase = request.keyword.trim();
    if phrase.is_empty() {
        return Err(AppError::Validation("keyword cannot be empty".to_string()));
    }
    let keyword_id = state
        .db
        .create_filter_keyword(&id, phrase, request.whole_word.unwrap_or(true))
        .await?;
    Ok(Json(filter_keyword_value(
        &keyword_id,
        phrase,
        request.whole_word.unwrap_or(true),
    )))
}

pub async fn update_filter_keyword(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(keyword_id): Path<String>,
    Json(request): Json<CreateFilterV2KeywordRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    let phrase = request.keyword.trim();
    if phrase.is_empty() {
        return Err(AppError::Validation("keyword cannot be empty".to_string()));
    }
    let existing = state
        .db
        .get_filter_keyword(&keyword_id)
        .await?
        .ok_or(AppError::NotFound)?;
    let whole_word = request.whole_word.unwrap_or(existing.3);
    let updated = state
        .db
        .update_filter_keyword(&keyword_id, phrase, whole_word)
        .await?;
    if !updated {
        return Err(AppError::NotFound);
    }
    Ok(Json(filter_keyword_value(&keyword_id, phrase, whole_word)))
}

pub async fn delete_filter_keyword(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(keyword_id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_filter_keyword(&keyword_id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({})))
}

pub async fn get_filter_statuses(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    Ok(Json(
        state
            .db
            .get_filter_statuses(&id)
            .await?
            .into_iter()
            .map(|(status_filter_id, status_id)| {
                serde_json::json!({
                    "id": status_filter_id,
                    "status_id": status_id,
                })
            })
            .collect(),
    ))
}

#[derive(Debug, Deserialize)]
pub struct CreateFilterStatusRequest {
    pub status_id: String,
}

pub async fn create_filter_status(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(request): Json<CreateFilterStatusRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    state.db.get_filter(&id).await?.ok_or(AppError::NotFound)?;
    let status_id = request.status_id.trim();
    if status_id.is_empty() {
        return Err(AppError::Validation("status_id is required".to_string()));
    }
    let filter_status_id = state.db.create_filter_status(&id, status_id).await?;
    Ok(Json(serde_json::json!({
        "id": filter_status_id,
        "status_id": status_id,
    })))
}

pub async fn delete_filter_status(
    State(state): State<FiltersApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    let deleted = state.db.delete_filter_status(&id).await?;
    if !deleted {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({})))
}
