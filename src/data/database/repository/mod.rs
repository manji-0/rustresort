mod account;
mod admin_reports;
mod conversations;
mod delivery_jobs;
mod domain_blocks;
mod featured_tags;
mod filters;
mod follow;
mod follow_requests;
mod interactions;
mod key_cache;
mod lists;
mod media;
mod moderation;
mod notifications;
mod oauth;
mod passkeys;
mod polls;
mod push_subscriptions;
mod remote_blocks;
mod remote_profiles;
mod remote_status_attachments;
mod remote_status_metadata;
mod scheduled_statuses;
mod search;
mod settings;
mod status;

async fn rollback_with_log(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Sqlite>,
    context: &'static str,
) {
    if let Err(rollback_error) = sqlx::query("ROLLBACK").execute(&mut **conn).await {
        tracing::warn!(
            %rollback_error,
            context,
            "SQLite transaction rollback failed"
        );
    }
}
