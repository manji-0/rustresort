use chrono::Utc;
use std::collections::HashSet;

use crate::data::{Account, EntityId, PersistedReason, ScheduledStatus, Status, StatusVisibility};
use crate::error::AppError;
use crate::metrics::POSTS_TOTAL;
use crate::{AppState, service::AccountService};

use super::StatusService;

const SCHEDULED_STATUS_BATCH_SIZE: usize = 16;
const SCHEDULED_STATUS_IDLE_MILLIS: u64 = 250;

fn should_deliver_to_followers_collection(visibility: StatusVisibility) -> bool {
    matches!(
        visibility,
        StatusVisibility::Public | StatusVisibility::Unlisted | StatusVisibility::Private
    )
}

fn build_status_service(state: &AppState) -> StatusService {
    StatusService::new(
        state.db.clone(),
        state.timeline_cache.clone(),
        state.storage.clone(),
        state.streaming_event_bus.clone(),
        state.config.server.base_url().to_string(),
        state.config.auth.username.clone(),
    )
}

fn build_account_service(state: &AppState) -> AccountService {
    AccountService::new(state.db.clone(), state.storage.clone())
}

fn build_delivery(state: &AppState, account: &Account) -> crate::federation::ActivityDelivery {
    crate::federation::build_local_delivery(
        state.http_client.clone(),
        &state.config.server.base_url(),
        account,
    )
}

fn parse_optional_string_array(
    raw: &Option<String>,
    operation: &'static str,
) -> Result<Vec<String>, AppError> {
    match raw {
        Some(value) => {
            serde_json::from_str(value).map_err(|error| AppError::serialization(operation, error))
        }
        None => Ok(Vec::new()),
    }
}

async fn resolve_reply_target(
    state: &AppState,
    status_service: &StatusService,
    in_reply_to_id: Option<&str>,
) -> Result<(Option<String>, Option<String>, PersistedReason), AppError> {
    let mut in_reply_to_uri = None;
    let mut reply_target_account_address = None;
    let mut persisted_reason = PersistedReason::Own;

    if let Some(in_reply_to_id) = in_reply_to_id {
        if let Some(reply_target) = status_service.find(in_reply_to_id).await? {
            in_reply_to_uri = Some(reply_target.uri.clone());
            if reply_target.is_local {
                persisted_reason = PersistedReason::ReplyToOwn;
            } else if !reply_target.account_address.is_empty() {
                reply_target_account_address = Some(reply_target.account_address);
            }
        } else if let Some(reply_target) = status_service.find_by_uri(in_reply_to_id).await? {
            in_reply_to_uri = Some(reply_target.uri.clone());
            if reply_target.is_local {
                persisted_reason = PersistedReason::ReplyToOwn;
            } else if !reply_target.account_address.is_empty() {
                reply_target_account_address = Some(reply_target.account_address);
            }
        } else if let Some(cached_target) = state.timeline_cache.get_by_uri(in_reply_to_id).await {
            in_reply_to_uri = Some(cached_target.uri.clone());
            if !cached_target.account_address.is_empty() {
                reply_target_account_address = Some(cached_target.account_address.clone());
            }
        } else {
            return Err(AppError::Validation(
                "scheduled in_reply_to_id does not exist".to_string(),
            ));
        }
    }

    Ok((
        in_reply_to_uri,
        reply_target_account_address,
        persisted_reason,
    ))
}

async fn resolve_quote_target(
    state: &AppState,
    status_service: &StatusService,
    quoted_status_id: Option<&str>,
) -> Result<Option<(String, Option<String>)>, AppError> {
    let Some(quoted_status_id) = quoted_status_id else {
        return Ok(None);
    };

    if let Some(quoted_target) = status_service.find(quoted_status_id).await? {
        return Ok(Some((
            quoted_target.uri,
            (!quoted_target.is_local && !quoted_target.account_address.is_empty())
                .then_some(quoted_target.account_address),
        )));
    }
    if let Some(quoted_target) = status_service.find_by_uri(quoted_status_id).await? {
        return Ok(Some((
            quoted_target.uri,
            (!quoted_target.is_local && !quoted_target.account_address.is_empty())
                .then_some(quoted_target.account_address),
        )));
    }
    if let Some(cached_target) = state.timeline_cache.get_by_uri(quoted_status_id).await {
        return Ok(Some((
            cached_target.uri.clone(),
            (!cached_target.account_address.is_empty())
                .then_some(cached_target.account_address.clone()),
        )));
    }
    if quoted_status_id.starts_with("http://") || quoted_status_id.starts_with("https://") {
        let quoted_target = status_service
            .ensure_remote_status_persisted(quoted_status_id, PersistedReason::Own)
            .await?;
        return Ok(Some((
            quoted_target.uri,
            (!quoted_target.account_address.is_empty()).then_some(quoted_target.account_address),
        )));
    }

    Err(AppError::Validation(
        "scheduled quoted_status_id does not exist".to_string(),
    ))
}

async fn publish_scheduled_status(
    state: &AppState,
    account: &Account,
    scheduled: &ScheduledStatus,
) -> Result<(), AppError> {
    let status_service = build_status_service(state);
    let account_service = build_account_service(state);

    let visibility = StatusVisibility::parse(&scheduled.visibility).ok_or_else(|| {
        AppError::Validation(
            "scheduled status visibility must be one of: public, unlisted, private, direct"
                .to_string(),
        )
    })?;
    let media_ids =
        parse_optional_string_array(&scheduled.media_ids, "scheduled media_ids deserialization")?;
    let poll_options = parse_optional_string_array(
        &scheduled.poll_options,
        "scheduled poll options deserialization",
    )?;
    let poll = if poll_options.is_empty() {
        None
    } else {
        Some((
            poll_options,
            scheduled.poll_expires_in.ok_or_else(|| {
                AppError::Validation("scheduled poll_expires_in is missing".to_string())
            })?,
            scheduled.poll_multiple,
        ))
    };

    let (in_reply_to_uri, reply_target_account_address, persisted_reason) =
        resolve_reply_target(state, &status_service, scheduled.in_reply_to_id.as_deref()).await?;
    let quote_target = resolve_quote_target(
        state,
        &status_service,
        scheduled.quoted_status_id.as_deref(),
    )
    .await?;
    let quote_of_uri = quote_target.as_ref().map(|(uri, _)| uri.clone());

    let status_id = EntityId::new_string();
    let status = Status {
        id: status_id.clone(),
        uri: format!(
            "{}/users/{}/statuses/{}",
            state.config.server.base_url(),
            account.username,
            status_id
        ),
        content: format!(
            "<p>{}</p>",
            html_escape::encode_text(&scheduled.status_text)
        ),
        content_warning: scheduled.content_warning.clone(),
        visibility,
        language: scheduled.language.clone().or(Some("en".to_string())),
        account_address: String::new(),
        is_local: true,
        in_reply_to_uri,
        boost_of_uri: None,
        quote_of_uri,
        persisted_reason,
        created_at: Utc::now(),
        fetched_at: None,
    };

    let mention_addresses =
        crate::api::mastodon::federation_delivery::extract_remote_mentions_from_content(
            &scheduled.status_text,
            &state.config.server.domain,
        );
    let mut explicit_addresses = mention_addresses.clone();
    if let Some(reply_target_account_address) = reply_target_account_address.clone() {
        explicit_addresses.push(reply_target_account_address);
    }
    if let Some((_, Some(quote_target_account_address))) = quote_target.clone() {
        explicit_addresses.push(quote_target_account_address);
    }
    let explicit_recipients =
        crate::api::mastodon::federation_delivery::resolve_remote_recipients_with_dependencies(
            state.db.as_ref(),
            state.profile_cache.as_ref(),
            state.federation_fetch_client.as_ref(),
            explicit_addresses,
        )
        .await;
    let mention_address_set = mention_addresses.into_iter().collect::<HashSet<_>>();
    let mention_tags = explicit_recipients
        .iter()
        .filter(|recipient| mention_address_set.contains(&recipient.address))
        .map(|recipient| {
            serde_json::json!({
                "type": "Mention",
                "href": recipient.actor_uri,
                "name": format!("@{}", recipient.address),
            })
        })
        .collect::<Vec<_>>();

    let follower_inboxes = if should_deliver_to_followers_collection(status.visibility) {
        match account_service.get_follower_inboxes().await {
            Ok(inboxes) => inboxes,
            Err(error) => {
                tracing::warn!(
                    %error,
                    scheduled_status_id = %scheduled.id,
                    "Skipping follower fan-out prefetch for scheduled Create delivery"
                );
                Vec::new()
            }
        }
    } else {
        Vec::new()
    };

    status_service
        .persist_local_status_with_media_and_poll(
            &status,
            &media_ids,
            poll.as_ref().map(|(options, expires_in, multiple)| {
                (options.as_slice(), *expires_in, *multiple)
            }),
        )
        .await?;
    POSTS_TOTAL.inc();

    let mut delivery_targets = follower_inboxes;
    let mut seen_delivery_targets = delivery_targets.iter().cloned().collect::<HashSet<_>>();
    for recipient in &explicit_recipients {
        if seen_delivery_targets.insert(recipient.inbox_uri.clone()) {
            delivery_targets.push(recipient.inbox_uri.clone());
        }
    }

    if !delivery_targets.is_empty() {
        let delivery = build_delivery(state, account);
        let explicit_recipient_actor_uris = explicit_recipients
            .iter()
            .map(|recipient| recipient.actor_uri.clone())
            .collect::<Vec<_>>();
        let _ = delivery
            .queue_create_with_audience(
                state.db.as_ref(),
                &status,
                delivery_targets,
                &explicit_recipient_actor_uris,
                &mention_tags,
            )
            .await;
    }

    Ok(())
}

pub async fn run_scheduled_statuses_once(state: &AppState) -> Result<usize, AppError> {
    let due = state
        .db
        .get_due_scheduled_statuses(SCHEDULED_STATUS_BATCH_SIZE)
        .await?;
    if due.is_empty() {
        return Ok(0);
    }

    let account = build_account_service(state).get_account().await?;
    for scheduled in &due {
        match publish_scheduled_status(state, &account, scheduled).await {
            Ok(()) => {
                state
                    .db
                    .mark_scheduled_status_published(&scheduled.id)
                    .await?
            }
            Err(error) => {
                tracing::error!(
                    %error,
                    scheduled_status_id = %scheduled.id,
                    "Scheduled status publish failed"
                );
                state
                    .db
                    .mark_scheduled_status_failed(&scheduled.id, &error.to_string())
                    .await?;
            }
        }
    }

    Ok(due.len())
}

pub fn spawn_scheduled_status_runner(state: AppState) {
    tokio::spawn(async move {
        loop {
            match run_scheduled_statuses_once(&state).await {
                Ok(0) => {
                    tokio::time::sleep(std::time::Duration::from_millis(
                        SCHEDULED_STATUS_IDLE_MILLIS,
                    ))
                    .await;
                }
                Ok(_) => tokio::task::yield_now().await,
                Err(error) => {
                    tracing::error!(%error, "Scheduled status runner iteration failed");
                    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                }
            }
        }
    });

    tracing::info!("Scheduled status runner spawned");
}
