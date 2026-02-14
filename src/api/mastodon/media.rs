//! Media endpoints

use axum::{
    extract::{Multipart, Path, State},
    response::Json,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::api::metrics::{
    DB_QUERIES_TOTAL, DB_QUERY_DURATION_SECONDS, HTTP_REQUEST_DURATION_SECONDS,
    HTTP_REQUESTS_TOTAL, MEDIA_BYTES_UPLOADED, MEDIA_UPLOADS_TOTAL,
};
use crate::auth::CurrentUser;
use crate::error::AppError;

/// Media attachment response
#[derive(Debug, Serialize)]
pub struct MediaAttachmentResponse {
    pub id: String,
    #[serde(rename = "type")]
    pub media_type: String,
    pub url: String,
    pub preview_url: String,
    pub remote_url: Option<String>,
    pub text_url: Option<String>,
    pub meta: MediaMeta,
    pub description: Option<String>,
    pub blurhash: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct MediaMeta {
    pub original: Option<MediaMetaInfo>,
    pub small: Option<MediaMetaInfo>,
}

#[derive(Debug, Serialize)]
pub struct MediaMetaInfo {
    pub width: Option<i32>,
    pub height: Option<i32>,
    pub size: Option<String>,
    pub aspect: Option<f64>,
}

/// POST /api/v1/media
pub async fn upload_media(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    mut multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    use crate::data::{EntityId, MediaAttachment};
    use chrono::Utc;

    // Start timing the request
    let _timer = HTTP_REQUEST_DURATION_SECONDS
        .with_label_values(&["POST", "/api/v1/media"])
        .start_timer();

    let mut file_data: Option<Vec<u8>> = None;
    let mut filename: Option<String> = None;
    let mut content_type: Option<String> = None;
    let mut description: Option<String> = None;

    // Parse multipart form data
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::Validation(format!("Failed to parse multipart: {}", e)))?
    {
        let field_name = field.name().unwrap_or("").to_string();

        match field_name.as_str() {
            "file" => {
                filename = field.file_name().map(|s| s.to_string());
                content_type = field.content_type().map(|s| s.to_string());
                file_data = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| AppError::Validation(format!("Failed to read file: {}", e)))?
                        .to_vec(),
                );
            }
            "description" => {
                description = Some(field.text().await.map_err(|e| {
                    AppError::Validation(format!("Failed to read description: {}", e))
                })?);
            }
            _ => {}
        }
    }

    let file_data = file_data.ok_or(AppError::Validation("No file provided".to_string()))?;
    let filename = filename.ok_or(AppError::Validation("No filename provided".to_string()))?;
    let content_type = content_type.unwrap_or_else(|| "application/octet-stream".to_string());

    // Validate file size (10MB for images, 40MB for videos)
    let max_size = if content_type.starts_with("image/") {
        10 * 1024 * 1024 // 10MB
    } else if content_type.starts_with("video/") {
        40 * 1024 * 1024 // 40MB
    } else {
        return Err(AppError::Validation("Unsupported media type".to_string()));
    };

    if file_data.len() > max_size {
        return Err(AppError::Validation(format!(
            "File too large: {} bytes (max: {} bytes)",
            file_data.len(),
            max_size
        )));
    }

    // Validate MIME type
    let supported_types = [
        "image/jpeg",
        "image/png",
        "image/gif",
        "image/webp",
        "video/mp4",
    ];

    if !supported_types.contains(&content_type.as_str()) {
        return Err(AppError::Validation(format!(
            "Unsupported MIME type: {}",
            content_type
        )));
    }

    // Generate media ID
    let media_id = EntityId::new().0;

    // Determine media type
    let media_type = if content_type.starts_with("image/") {
        "image"
    } else if content_type.starts_with("video/") {
        "video"
    } else {
        "unknown"
    };

    // Generate file path for R2 storage
    let file_extension = filename.split('.').last().unwrap_or("bin");
    let s3_key = format!("media/{}.{}", media_id, file_extension);

    // Upload to R2 storage
    let url = state
        .storage
        .upload(&s3_key, file_data.clone(), &content_type)
        .await
        .map_err(|e| AppError::Storage(format!("Failed to upload media: {}", e)))?;

    // TODO: Generate thumbnail for images
    let thumbnail_s3_key = None;
    let thumbnail_url = url.clone();

    // Create media attachment record
    let media = MediaAttachment {
        id: media_id.clone(),
        status_id: None, // Not yet attached to a status
        s3_key: s3_key.clone(),
        thumbnail_s3_key,
        content_type: content_type.clone(),
        file_size: file_data.len() as i64,
        description: description.clone(),
        blurhash: None, // TODO: Generate blurhash
        width: None,    // TODO: Extract image/video dimensions
        height: None,
        created_at: Utc::now(),
    };

    // Save to database
    let db_timer = DB_QUERY_DURATION_SECONDS
        .with_label_values(&["INSERT", "media"])
        .start_timer();
    state.db.insert_media(&media).await?;
    DB_QUERIES_TOTAL
        .with_label_values(&["INSERT", "media"])
        .inc();
    db_timer.observe_duration();

    // Update media metrics
    MEDIA_UPLOADS_TOTAL.inc();
    MEDIA_BYTES_UPLOADED.inc_by(file_data.len() as f64);

    // Return response
    let response = MediaAttachmentResponse {
        id: media.id,
        media_type: media_type.to_string(),
        url,
        preview_url: thumbnail_url,
        remote_url: None,
        text_url: None,
        meta: MediaMeta {
            original: media.width.and_then(|w| {
                media.height.map(|h| MediaMetaInfo {
                    width: Some(w),
                    height: Some(h),
                    size: Some(format!("{}x{}", w, h)),
                    aspect: Some(w as f64 / h as f64),
                })
            }),
            small: None,
        },
        description: media.description,
        blurhash: media.blurhash,
    };

    // Record successful request
    HTTP_REQUESTS_TOTAL
        .with_label_values(&["POST", "/api/v1/media", "200"])
        .inc();

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// POST /api/v2/media (async upload)
pub async fn upload_media_v2(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    multipart: Multipart,
) -> Result<Json<serde_json::Value>, AppError> {
    // For now, v2 is the same as v1 (synchronous upload)
    // In a full implementation, v2 would return immediately with a processing status
    // and the client would poll for completion
    upload_media(State(state), CurrentUser(_session), multipart).await
}

/// GET /api/v1/media/:id
pub async fn get_media(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get media from database
    let media = state.db.get_media(&id).await?.ok_or(AppError::NotFound)?;

    // Generate URLs
    let url = state.storage.get_public_url(&media.s3_key);
    let preview_url = if let Some(ref thumb_key) = media.thumbnail_s3_key {
        state.storage.get_public_url(thumb_key)
    } else {
        url.clone()
    };

    // Determine media type from content type
    let media_type = if media.content_type.starts_with("image/") {
        "image"
    } else if media.content_type.starts_with("video/") {
        "video"
    } else {
        "unknown"
    };

    // Build response
    let response = MediaAttachmentResponse {
        id: media.id,
        media_type: media_type.to_string(),
        url,
        preview_url,
        remote_url: None,
        text_url: None,
        meta: MediaMeta {
            original: media.width.and_then(|w| {
                media.height.map(|h| MediaMetaInfo {
                    width: Some(w),
                    height: Some(h),
                    size: Some(format!("{}x{}", w, h)),
                    aspect: Some(w as f64 / h as f64),
                })
            }),
            small: None,
        },
        description: media.description,
        blurhash: media.blurhash,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

/// PUT /api/v1/media/:id
pub async fn update_media(
    State(state): State<AppState>,
    CurrentUser(_session): CurrentUser,
    Path(id): Path<String>,
    Json(req): Json<UpdateMediaRequest>,
) -> Result<Json<serde_json::Value>, AppError> {
    // Get media from database
    let mut media = state.db.get_media(&id).await?.ok_or(AppError::NotFound)?;

    // Update description if provided
    if let Some(description) = req.description {
        media.description = Some(description);
    }

    // TODO: Handle focus point update
    // Focus point format: "x,y" where x and y are floats between -1.0 and 1.0

    // Update in database
    state.db.update_media(&media).await?;

    // Generate URLs
    let url = state.storage.get_public_url(&media.s3_key);
    let preview_url = if let Some(ref thumb_key) = media.thumbnail_s3_key {
        state.storage.get_public_url(thumb_key)
    } else {
        url.clone()
    };

    // Determine media type from content type
    let media_type = if media.content_type.starts_with("image/") {
        "image"
    } else if media.content_type.starts_with("video/") {
        "video"
    } else {
        "unknown"
    };

    // Build response
    let response = MediaAttachmentResponse {
        id: media.id,
        media_type: media_type.to_string(),
        url,
        preview_url,
        remote_url: None,
        text_url: None,
        meta: MediaMeta {
            original: media.width.and_then(|w| {
                media.height.map(|h| MediaMetaInfo {
                    width: Some(w),
                    height: Some(h),
                    size: Some(format!("{}x{}", w, h)),
                    aspect: Some(w as f64 / h as f64),
                })
            }),
            small: None,
        },
        description: media.description,
        blurhash: media.blurhash,
    };

    Ok(Json(serde_json::to_value(response).unwrap()))
}

#[derive(Debug, Deserialize)]
pub struct UpdateMediaRequest {
    pub description: Option<String>,
    pub focus: Option<String>,
}
