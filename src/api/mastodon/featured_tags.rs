use axum::{
    extract::{Form, Path, State},
    response::Json,
};
use serde::Deserialize;

use crate::{AccountApiState, api::dto::FeaturedTagResponse, auth::CurrentUser, error::AppError};

const MAX_FEATURED_TAGS: i64 = 10;

#[derive(Debug, Deserialize)]
pub struct FeatureTagRequest {
    pub name: String,
}

fn featured_tag_response(
    profile_url_prefix: &str,
    row: (String, String, i64, Option<String>),
) -> FeaturedTagResponse {
    let (id, name, statuses_count, last_status_at) = row;
    FeaturedTagResponse {
        id,
        name: name.clone(),
        url: format!("{profile_url_prefix}/tagged/{name}"),
        statuses_count,
        last_status_at,
    }
}

pub async fn get_featured_tags(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<FeaturedTagResponse>>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let profile_url_prefix =
        crate::api::local_profile_url(&state.config.server.base_url(), &account.username);
    let rows = state.db.list_featured_tags().await?;
    Ok(Json(
        rows.into_iter()
            .map(|row| featured_tag_response(&profile_url_prefix, row))
            .collect(),
    ))
}

pub async fn feature_tag(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Form(req): Form<FeatureTagRequest>,
) -> Result<Json<FeaturedTagResponse>, AppError> {
    let account = state.db.get_account().await?.ok_or(AppError::NotFound)?;
    let profile_url_prefix =
        crate::api::local_profile_url(&state.config.server.base_url(), &account.username);
    if state.db.count_featured_tags().await? >= MAX_FEATURED_TAGS
        && state
            .db
            .get_featured_tag_by_name(&req.name)
            .await?
            .is_none()
    {
        return Err(AppError::Validation(
            "Validation failed: Featured tags limit reached".to_string(),
        ));
    }

    let row = state.db.create_featured_tag(&req.name).await?;
    Ok(Json(featured_tag_response(&profile_url_prefix, row)))
}

pub async fn unfeature_tag(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    if !state.db.delete_featured_tag(&id).await? {
        return Err(AppError::NotFound);
    }
    Ok(Json(serde_json::json!({})))
}

pub async fn featured_tag_suggestions(
    State(state): State<AccountApiState>,
    CurrentUser(_session): CurrentUser,
) -> Result<Json<Vec<serde_json::Value>>, AppError> {
    let tags = state
        .db
        .suggested_featured_tags(MAX_FEATURED_TAGS as usize)
        .await?;
    Ok(Json(
        tags.into_iter()
            .map(|(id, name, _statuses_count, _last_status_at)| {
                serde_json::json!({
                    "id": id,
                    "name": name,
                    "url": format!("{}/tags/{}", state.config.server.base_url(), name),
                    "history": [],
                    "following": false
                })
            })
            .collect(),
    ))
}
