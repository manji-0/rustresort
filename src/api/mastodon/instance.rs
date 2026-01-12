//! Instance endpoints

use axum::{extract::State, response::Json};

use crate::AppState;

/// GET /api/v1/instance
pub async fn instance(State(state): State<AppState>) -> Json<serde_json::Value> {
    use crate::api::dto::*;

    let _base_url = state.config.server.base_url();

    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        Some(crate::api::account_to_response(&account, &state.config))
    } else {
        None
    };

    // Get stats
    let user_count = 1; // Single-user instance
    let status_count = state
        .db
        .get_local_statuses(1000, None)
        .await
        .map(|s| s.len() as i64)
        .unwrap_or(0);
    let domain_count = 0; // TODO: Count federated domains

    let response = InstanceResponse {
        uri: state.config.server.domain.clone(),
        title: state.config.instance.title.clone(),
        short_description: state.config.instance.description.clone(),
        description: state.config.instance.description.clone(),
        email: state.config.instance.contact_email.clone(),
        version: format!("RustResort {}", env!("CARGO_PKG_VERSION")),
        languages: vec!["en".to_string()],
        registrations: false, // Single-user instance
        approval_required: false,
        invites_enabled: false,
        configuration: InstanceConfiguration {
            statuses: StatusesConfiguration {
                max_characters: 500,
                max_media_attachments: 4,
                characters_reserved_per_url: 23,
            },
            media_attachments: MediaConfiguration {
                supported_mime_types: vec![
                    "image/jpeg".to_string(),
                    "image/png".to_string(),
                    "image/gif".to_string(),
                    "image/webp".to_string(),
                    "video/mp4".to_string(),
                ],
                image_size_limit: 10485760,   // 10MB
                image_matrix_limit: 16777216, // 4096x4096
                video_size_limit: 41943040,   // 40MB
                video_frame_rate_limit: 60,
                video_matrix_limit: 2304000, // 1920x1200
            },
            polls: PollsConfiguration {
                max_options: 4,
                max_characters_per_option: 50,
                min_expiration: 300,     // 5 minutes
                max_expiration: 2629746, // 1 month
            },
        },
        urls: InstanceUrls {
            streaming_api: format!("wss://{}", state.config.server.domain),
        },
        stats: InstanceStats {
            user_count,
            status_count,
            domain_count,
        },
        thumbnail: None,
        contact_account,
    };

    Json(serde_json::to_value(response).unwrap())
}

/// GET /api/v1/instance/peers - Get instance peers
///
/// List of federated instances this instance knows about.
pub async fn instance_peers(State(_state): State<AppState>) -> Json<serde_json::Value> {
    // TODO: Implement federated peers tracking
    // For single-user instance, return empty array for now
    Json(serde_json::json!([]))
}

/// GET /api/v1/instance/activity - Get instance activity
///
/// Instance activity over the last 3 months, binned weekly.
pub async fn instance_activity(State(_state): State<AppState>) -> Json<serde_json::Value> {
    // Return activity statistics for the last 12 weeks
    // For single-user instance, return minimal activity data

    let mut activity = Vec::new();
    let now = chrono::Utc::now();

    for i in 0..12 {
        let week_start = now - chrono::Duration::weeks(11 - i);
        activity.push(serde_json::json!({
            "week": week_start.timestamp().to_string(),
            "statuses": "0",
            "logins": "0",
            "registrations": "0"
        }));
    }

    Json(serde_json::json!(activity))
}

/// GET /api/v1/instance/rules - Get instance rules
///
/// List of rules for this instance.
pub async fn instance_rules(State(_state): State<AppState>) -> Json<serde_json::Value> {
    // TODO: Make rules configurable
    // For now, return basic rules for single-user instance
    Json(serde_json::json!([
        {
            "id": "1",
            "text": "Be respectful and civil in all interactions."
        },
        {
            "id": "2",
            "text": "No spam, harassment, or illegal content."
        },
        {
            "id": "3",
            "text": "Content warnings are required for sensitive material."
        }
    ]))
}

/// GET /api/v2/instance - Get instance information (v2)
///
/// Extended instance information with additional fields.
pub async fn instance_v2(State(state): State<AppState>) -> Json<serde_json::Value> {
    // Get account for contact
    let contact_account = if let Ok(Some(account)) = state.db.get_account().await {
        Some(crate::api::account_to_response(&account, &state.config))
    } else {
        None
    };

    // Get stats
    let _user_count = 1; // Single-user instance
    let _status_count = state
        .db
        .get_local_statuses(1000, None)
        .await
        .map(|s| s.len() as i64)
        .unwrap_or(0);
    let _domain_count = 0; // TODO: Count federated domains

    Json(serde_json::json!({
        "domain": state.config.server.domain,
        "title": state.config.instance.title,
        "version": format!("RustResort {}", env!("CARGO_PKG_VERSION")),
        "source_url": "https://github.com/yourusername/rustresort",
        "description": state.config.instance.description,
        "usage": {
            "users": {
                "active_month": 1
            }
        },
        "thumbnail": {
            "url": null,
            "blurhash": null,
            "versions": {}
        },
        "languages": ["en"],
        "configuration": {
            "urls": {
                "streaming": format!("wss://{}", state.config.server.domain)
            },
            "accounts": {
                "max_featured_tags": 10
            },
            "statuses": {
                "max_characters": 500,
                "max_media_attachments": 4,
                "characters_reserved_per_url": 23
            },
            "media_attachments": {
                "supported_mime_types": [
                    "image/jpeg",
                    "image/png",
                    "image/gif",
                    "image/webp",
                    "video/mp4"
                ],
                "image_size_limit": 10485760,
                "image_matrix_limit": 16777216,
                "video_size_limit": 41943040,
                "video_frame_rate_limit": 60,
                "video_matrix_limit": 2304000
            },
            "polls": {
                "max_options": 4,
                "max_characters_per_option": 50,
                "min_expiration": 300,
                "max_expiration": 2629746
            },
            "translation": {
                "enabled": false
            }
        },
        "registrations": {
            "enabled": false,
            "approval_required": false,
            "message": null
        },
        "contact": {
            "email": state.config.instance.contact_email,
            "account": contact_account
        },
        "rules": [
            {
                "id": "1",
                "text": "Be respectful and civil in all interactions."
            },
            {
                "id": "2",
                "text": "No spam, harassment, or illegal content."
            },
            {
                "id": "3",
                "text": "Content warnings are required for sensitive material."
            }
        ]
    }))
}
