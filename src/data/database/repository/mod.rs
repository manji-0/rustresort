mod account;
mod conversations;
mod domain_blocks;
mod filters;
mod follow;
mod follow_requests;
mod interactions;
mod lists;
mod media;
mod moderation;
mod notifications;
mod oauth;
mod polls;
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
