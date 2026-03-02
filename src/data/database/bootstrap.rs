use super::*;
use sqlx::{Pool, Sqlite};

const OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY: &str =
    "oauth_tokens_access_token_hash_migration";
const OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE: &str = "done";

pub(super) async fn migrate_legacy_oauth_tokens(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    let migration_state =
        sqlx::query_scalar::<_, String>("SELECT value FROM settings WHERE key = ?")
            .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY)
            .fetch_optional(pool)
            .await?;

    if migration_state.as_deref() == Some(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE) {
        return Ok(());
    }

    let legacy_rows =
        sqlx::query_as::<_, (String, String)>("SELECT id, access_token FROM oauth_tokens")
            .fetch_all(pool)
            .await?;

    if legacy_rows.is_empty() {
        return Ok(());
    }

    let mut tx = pool.begin().await?;
    let mut migrated_count = 0usize;

    for (id, stored_access_token) in legacy_rows {
        if is_hashed_oauth_access_token(&stored_access_token) {
            continue;
        }

        let hashed_access_token = hash_oauth_access_token(&stored_access_token);
        sqlx::query("UPDATE oauth_tokens SET access_token = ? WHERE id = ?")
            .bind(&hashed_access_token)
            .bind(&id)
            .execute(&mut *tx)
            .await?;
        migrated_count += 1;
    }

    sqlx::query(
        "INSERT INTO settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
    )
    .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_SETTING_KEY)
    .bind(OAUTH_ACCESS_TOKEN_HASH_MIGRATION_DONE)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    if migrated_count > 0 {
        tracing::info!(
            migrated_count,
            "Migrated legacy OAuth access tokens to hashed storage"
        );
    }

    Ok(())
}

pub(super) async fn backfill_missing_status_hashtags(pool: &Pool<Sqlite>) -> Result<(), AppError> {
    const HASHTAG_BACKFILL_BATCH_SIZE: i64 = 250;

    let mut backfilled_count = 0usize;
    let mut last_status_id: Option<String> = None;

    loop {
        let statuses = if let Some(last_status_id) = last_status_id.as_deref() {
            sqlx::query_as::<_, (String, String)>(
                r#"
                SELECT s.id, s.content
                FROM statuses s
                WHERE s.id > ?
                  AND s.content LIKE '%#%'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM status_hashtags sh
                      WHERE sh.status_id = s.id
                  )
                ORDER BY s.id ASC
                LIMIT ?
                "#,
            )
            .bind(last_status_id)
            .bind(HASHTAG_BACKFILL_BATCH_SIZE)
            .fetch_all(pool)
            .await?
        } else {
            sqlx::query_as::<_, (String, String)>(
                r#"
                SELECT s.id, s.content
                FROM statuses s
                WHERE s.content LIKE '%#%'
                  AND NOT EXISTS (
                      SELECT 1
                      FROM status_hashtags sh
                      WHERE sh.status_id = s.id
                  )
                ORDER BY s.id ASC
                LIMIT ?
                "#,
            )
            .bind(HASHTAG_BACKFILL_BATCH_SIZE)
            .fetch_all(pool)
            .await?
        };

        if statuses.is_empty() {
            break;
        }
        backfilled_count += statuses.len();
        last_status_id = statuses.last().map(|(status_id, _)| status_id.clone());

        let mut tx = pool.begin().await?;
        for (status_id, content) in statuses {
            for hashtag in extract_hashtags_from_content(&content) {
                let hashtag_insert_id = EntityId::new_string();
                sqlx::query(
                    "INSERT OR IGNORE INTO hashtags (id, name, created_at) VALUES (?, ?, datetime('now'))",
                )
                .bind(&hashtag_insert_id)
                .bind(&hashtag)
                .execute(&mut *tx)
                .await?;

                let hashtag_id = sqlx::query_scalar::<_, String>(
                    "SELECT id FROM hashtags WHERE name = ? COLLATE NOCASE LIMIT 1",
                )
                .bind(&hashtag)
                .fetch_one(&mut *tx)
                .await?;

                let status_hashtag_id = EntityId::new_string();
                sqlx::query(
                    "INSERT OR IGNORE INTO status_hashtags (id, status_id, hashtag_id, created_at) VALUES (?, ?, ?, datetime('now'))",
                )
                .bind(&status_hashtag_id)
                .bind(&status_id)
                .bind(&hashtag_id)
                .execute(&mut *tx)
                .await?;
            }
        }
        tx.commit().await?;
    }

    if backfilled_count > 0 {
        tracing::info!(
            backfilled_count,
            "Backfilled missing hashtag index rows for existing statuses"
        );
    }

    Ok(())
}
