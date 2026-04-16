//! Database tests

use super::*;
use chrono::Utc;
use sqlx::{Row, SqlitePool};
use std::sync::Arc;
use tempfile::TempDir;
use tokio::sync::Barrier;

/// Helper to create a test database
async fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    (db, temp_dir)
}

fn test_db_connection_string(temp_dir: &TempDir) -> String {
    format!(
        "sqlite:{}?mode=rw",
        temp_dir.path().join("test.db").display()
    )
}

fn test_oauth_app() -> OAuthApp {
    OAuthApp {
        id: EntityId::new_string(),
        name: "Test App".to_string(),
        website: None,
        redirect_uri: "https://example.com/callback".to_string(),
        client_id: EntityId::new_string(),
        client_secret: EntityId::new_string(),
        vapid_key: Some(EntityId::new_string()),
        scopes: "read write".to_string(),
        created_at: Utc::now(),
    }
}

fn test_oauth_token(app_id: &str, access_token: &str) -> OAuthToken {
    OAuthToken {
        id: EntityId::new_string(),
        app_id: app_id.to_string(),
        access_token: access_token.to_string(),
        refresh_token: None,
        grant_type: "authorization_code".to_string(),
        scopes: "read write".to_string(),
        created_at: Utc::now(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        refresh_expires_at: None,
        revoked: false,
    }
}

async fn oauth_hash_migration_state(pool: &SqlitePool) -> String {
    sqlx::query_scalar::<_, String>(
        "SELECT value FROM settings WHERE key = 'oauth_tokens_access_token_hash_migration'",
    )
    .fetch_one(pool)
    .await
    .unwrap()
}

#[tokio::test]
async fn test_database_connection() {
    let (_db, _temp_dir) = create_test_db().await;
    // Connection successful if we get here without panicking
}

#[tokio::test]
async fn test_remote_profiles_persist_across_reopen() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    let now = Utc::now();

    db.upsert_remote_profile(&RemoteProfile {
        address: "bob@remote.example".to_string(),
        uri: "https://remote.example/users/bob".to_string(),
        display_name: Some("Bob".to_string()),
        note: Some("cached".to_string()),
        profile_fields_json: None,
        locked: true,
        bot: true,
        discoverable: false,
        indexable: false,
        avatar_url: Some("https://remote.example/avatar.png".to_string()),
        header_url: None,
        public_key_pem: "test-key".to_string(),
        inbox_uri: "https://remote.example/inbox".to_string(),
        outbox_uri: Some("https://remote.example/outbox".to_string()),
        followers_count: Some(12),
        following_count: Some(34),
        fetched_at: now,
        created_at: now,
        updated_at: now,
    })
    .await
    .unwrap();
    drop(db);

    let reopened = Database::connect(&db_path).await.unwrap();
    let profiles = reopened.list_remote_profiles().await.unwrap();
    let profile = profiles
        .iter()
        .find(|profile| profile.address == "bob@remote.example")
        .expect("persisted remote profile should exist");

    assert_eq!(profile.uri, "https://remote.example/users/bob");
    assert_eq!(profile.display_name.as_deref(), Some("Bob"));
    assert_eq!(profile.note.as_deref(), Some("cached"));
    assert_eq!(
        profile.avatar_url.as_deref(),
        Some("https://remote.example/avatar.png")
    );
    assert!(profile.locked);
    assert!(profile.bot);
    assert!(!profile.discoverable);
    assert!(!profile.indexable);
    assert_eq!(profile.inbox_uri, "https://remote.example/inbox");
    assert_eq!(profile.followers_count, Some(12));
    assert_eq!(profile.following_count, Some(34));
}

#[tokio::test]
async fn test_delivery_job_enqueue_claim_and_mark_delivered() {
    let (db, _temp_dir) = create_test_db().await;

    db.enqueue_delivery_job(
        "https://remote.example/inbox",
        r#"{"type":"Follow"}"#,
        "https://test.example/users/testuser#main-key",
    )
    .await
    .unwrap();

    let claimed = db.claim_pending_delivery_jobs(10).await.unwrap();
    assert_eq!(claimed.len(), 1);
    assert_eq!(claimed[0].inbox_url, "https://remote.example/inbox");
    assert!(claimed[0].claimed_at.is_some());

    let second_claim = db.claim_pending_delivery_jobs(10).await.unwrap();
    assert!(second_claim.is_empty());

    db.mark_delivery_job_delivered(&claimed[0].id)
        .await
        .unwrap();

    let third_claim = db.claim_pending_delivery_jobs(10).await.unwrap();
    assert!(third_claim.is_empty());
}

#[tokio::test]
async fn test_delivery_job_mark_failed_increments_attempts_and_clears_claim() {
    let (db, temp_dir) = create_test_db().await;

    db.enqueue_delivery_job(
        "https://remote.example/inbox",
        r#"{"type":"Like"}"#,
        "https://test.example/users/testuser#main-key",
    )
    .await
    .unwrap();

    let claimed = db.claim_pending_delivery_jobs(10).await.unwrap();
    assert_eq!(claimed.len(), 1);

    db.mark_delivery_job_failed(&claimed[0].id, "temporary failure")
        .await
        .unwrap();

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let row = sqlx::query(
        "SELECT attempts, last_error, claimed_at, delivered_at FROM delivery_jobs WHERE id = ?",
    )
    .bind(&claimed[0].id)
    .fetch_one(&pool)
    .await
    .unwrap();

    assert_eq!(row.get::<i64, _>("attempts"), 1);
    assert_eq!(row.get::<String, _>("last_error"), "temporary failure");
    assert_eq!(row.get::<Option<String>, _>("claimed_at"), None);
    assert_eq!(row.get::<Option<String>, _>("delivered_at"), None);
}

#[tokio::test]
async fn test_delivery_job_reap_dead_jobs_removes_exhausted_rows() {
    let (db, temp_dir) = create_test_db().await;

    db.enqueue_delivery_job(
        "https://remote.example/inbox",
        r#"{"type":"Undo"}"#,
        "https://test.example/users/testuser#main-key",
    )
    .await
    .unwrap();

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    sqlx::query("UPDATE delivery_jobs SET attempts = 8")
        .execute(&pool)
        .await
        .unwrap();

    let reaped = db.reap_dead_delivery_jobs(8).await.unwrap();
    assert_eq!(reaped, 1);

    let remaining = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM delivery_jobs")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(remaining, 0);
}

#[tokio::test]
async fn test_due_scheduled_statuses_excludes_failed_and_published_rows() {
    let (db, temp_dir) = create_test_db().await;

    db.create_scheduled_status(&ScheduledStatusInsert {
        scheduled_at: "2020-01-01T00:00:00Z".to_string(),
        status_text: "publish me".to_string(),
        visibility: "public".to_string(),
        content_warning: None,
        in_reply_to_id: None,
        quoted_status_id: None,
        media_ids: None,
        poll_options: None,
        poll_expires_in: None,
        poll_multiple: false,
        language: Some("en".to_string()),
    })
    .await
    .unwrap();
    let failed_id = db
        .create_scheduled_status(&ScheduledStatusInsert {
            scheduled_at: "2020-01-01T00:00:01Z".to_string(),
            status_text: "fail me".to_string(),
            visibility: "public".to_string(),
            content_warning: None,
            in_reply_to_id: None,
            quoted_status_id: None,
            media_ids: None,
            poll_options: None,
            poll_expires_in: None,
            poll_multiple: false,
            language: None,
        })
        .await
        .unwrap();
    let published_id = db
        .create_scheduled_status(&ScheduledStatusInsert {
            scheduled_at: "2020-01-01T00:00:02Z".to_string(),
            status_text: "done".to_string(),
            visibility: "public".to_string(),
            content_warning: None,
            in_reply_to_id: None,
            quoted_status_id: None,
            media_ids: None,
            poll_options: None,
            poll_expires_in: None,
            poll_multiple: false,
            language: None,
        })
        .await
        .unwrap();

    db.mark_scheduled_status_failed(&failed_id, "boom")
        .await
        .unwrap();
    db.mark_scheduled_status_published(&published_id)
        .await
        .unwrap();

    let due = db.get_due_scheduled_statuses(10).await.unwrap();
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].status_text, "publish me");

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let failed_error = sqlx::query_scalar::<_, Option<String>>(
        "SELECT error FROM scheduled_statuses WHERE id = ?",
    )
    .bind(&failed_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(failed_error.as_deref(), Some("boom"));
}

#[tokio::test]
async fn test_oauth_token_storage_hashes_access_token_and_lookup_uses_plain_token() {
    let (db, temp_dir) = create_test_db().await;

    let app = test_oauth_app();
    db.insert_oauth_app(&app).await.unwrap();

    let raw_access_token = "plain-oauth-token";
    let token = test_oauth_token(&app.id, raw_access_token);
    db.insert_oauth_token(&token).await.unwrap();

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let stored_access_token =
        sqlx::query_scalar::<_, String>("SELECT access_token FROM oauth_tokens WHERE id = ?")
            .bind(&token.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_ne!(stored_access_token, raw_access_token);
    assert!(stored_access_token.starts_with("sha256:"));

    let looked_up = db.get_oauth_token(raw_access_token).await.unwrap();
    assert!(looked_up.is_some());
    assert_eq!(looked_up.unwrap().id, token.id);
    assert!(
        db.get_oauth_token(&stored_access_token)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_oauth_token_migration_hashes_existing_plaintext_rows_on_reconnect() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let db = Database::connect(&db_path).await.unwrap();
    let app = test_oauth_app();
    db.insert_oauth_app(&app).await.unwrap();
    drop(db);

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();

    let legacy_token = test_oauth_token(&app.id, "legacy-plaintext-token");
    sqlx::query(
        r#"
        INSERT INTO oauth_tokens (
            id, app_id, access_token, grant_type, scopes, created_at, expires_at, revoked
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&legacy_token.id)
    .bind(&legacy_token.app_id)
    .bind(&legacy_token.access_token)
    .bind(&legacy_token.grant_type)
    .bind(&legacy_token.scopes)
    .bind(legacy_token.created_at)
    .bind(legacy_token.expires_at)
    .bind(legacy_token.revoked)
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let db = Database::connect(&db_path).await.unwrap();
    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let migrated_access_token =
        sqlx::query_scalar::<_, String>("SELECT access_token FROM oauth_tokens WHERE id = ?")
            .bind(&legacy_token.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_ne!(migrated_access_token, legacy_token.access_token);
    assert!(migrated_access_token.starts_with("sha256:"));

    let looked_up = db
        .get_oauth_token(&legacy_token.access_token)
        .await
        .unwrap();
    assert!(looked_up.is_some());
    assert_eq!(looked_up.unwrap().id, legacy_token.id);
    assert_eq!(oauth_hash_migration_state(&pool).await, "done");
}

#[tokio::test]
async fn test_oauth_token_migration_rehashes_fake_sha256_prefixed_plaintext() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");

    let db = Database::connect(&db_path).await.unwrap();
    let app = test_oauth_app();
    db.insert_oauth_app(&app).await.unwrap();
    drop(db);

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();

    let fake_prefixed_plaintext = "sha256:not-a-real-base64url-digest";
    let legacy_token = test_oauth_token(&app.id, fake_prefixed_plaintext);
    sqlx::query(
        r#"
        INSERT INTO oauth_tokens (
            id, app_id, access_token, grant_type, scopes, created_at, expires_at, revoked
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&legacy_token.id)
    .bind(&legacy_token.app_id)
    .bind(&legacy_token.access_token)
    .bind(&legacy_token.grant_type)
    .bind(&legacy_token.scopes)
    .bind(legacy_token.created_at)
    .bind(legacy_token.expires_at)
    .bind(legacy_token.revoked)
    .execute(&pool)
    .await
    .unwrap();
    drop(pool);

    let db = Database::connect(&db_path).await.unwrap();
    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let migrated_access_token =
        sqlx::query_scalar::<_, String>("SELECT access_token FROM oauth_tokens WHERE id = ?")
            .bind(&legacy_token.id)
            .fetch_one(&pool)
            .await
            .unwrap();

    assert_ne!(migrated_access_token, fake_prefixed_plaintext);
    assert!(migrated_access_token.starts_with("sha256:"));

    let looked_up = db.get_oauth_token(fake_prefixed_plaintext).await.unwrap();
    assert!(looked_up.is_some());
    assert_eq!(looked_up.unwrap().id, legacy_token.id);
    assert_eq!(oauth_hash_migration_state(&pool).await, "done");
}

#[tokio::test]
async fn test_oauth_token_revoke_works_with_hashed_storage() {
    let (db, temp_dir) = create_test_db().await;

    let app = test_oauth_app();
    db.insert_oauth_app(&app).await.unwrap();

    let raw_access_token = "revokable-token";
    let token = test_oauth_token(&app.id, raw_access_token);
    db.insert_oauth_token(&token).await.unwrap();

    db.revoke_oauth_token(raw_access_token).await.unwrap();
    assert!(
        db.get_oauth_token(raw_access_token)
            .await
            .unwrap()
            .is_none()
    );

    let pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    let revoked = sqlx::query_scalar::<_, i64>("SELECT revoked FROM oauth_tokens WHERE id = ?")
        .bind(&token.id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(revoked, 1);
}

#[tokio::test]
async fn test_get_or_create_conversation_reuses_existing_participant_set() {
    let (db, _temp_dir) = create_test_db().await;

    let first_id = db
        .get_or_create_conversation(&[
            "testuser@test.example.com".to_string(),
            "alice@remote.example".to_string(),
        ])
        .await
        .unwrap();
    let second_id = db
        .get_or_create_conversation(&[
            "alice@remote.example".to_string(),
            "testuser@test.example.com".to_string(),
        ])
        .await
        .unwrap();

    assert_eq!(first_id, second_id);
}

#[tokio::test]
async fn test_get_or_create_conversation_reuses_existing_participant_set_case_insensitively() {
    let (db, _temp_dir) = create_test_db().await;

    let first_id = db
        .get_or_create_conversation(&[
            "testuser@test.example.com".to_string(),
            "alice@remote.example".to_string(),
        ])
        .await
        .unwrap();
    let second_id = db
        .get_or_create_conversation(&[
            "TestUser@Test.Example.Com".to_string(),
            "Alice@Remote.Example".to_string(),
        ])
        .await
        .unwrap();

    assert_eq!(first_id, second_id);
}

#[tokio::test]
async fn test_oauth_token_lookup_rejects_expired_tokens() {
    let (db, _temp_dir) = create_test_db().await;

    let app = test_oauth_app();
    db.insert_oauth_app(&app).await.unwrap();

    let raw_access_token = "expired-token";
    let mut token = test_oauth_token(&app.id, raw_access_token);
    token.expires_at = Utc::now() - chrono::Duration::seconds(1);
    db.insert_oauth_token(&token).await.unwrap();

    assert!(
        db.get_oauth_token(raw_access_token)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_account_upsert_and_get() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new_string(),
        username: "testuser".to_string(),
        display_name: Some("Test User".to_string()),
        note: Some("Test bio".to_string()),
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "test_private_key".to_string(),
        public_key_pem: "test_public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    // Insert account
    db.upsert_account(&account).await.unwrap();

    // Retrieve account
    let retrieved = db.get_account().await.unwrap();
    assert!(retrieved.is_some());
    let retrieved = retrieved.unwrap();
    assert_eq!(retrieved.username, "testuser");
    assert_eq!(retrieved.display_name, Some("Test User".to_string()));
}

#[tokio::test]
async fn test_insert_account_if_empty_enforces_singleton() {
    let (db, _temp_dir) = create_test_db().await;

    let first = Account {
        id: EntityId::new_string(),
        username: "first".to_string(),
        display_name: Some("First".to_string()),
        note: None,
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "first_private_key".to_string(),
        public_key_pem: "first_public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let second = Account {
        id: EntityId::new_string(),
        username: "second".to_string(),
        display_name: Some("Second".to_string()),
        note: None,
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "second_private_key".to_string(),
        public_key_pem: "second_public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let inserted_first = db.insert_account_if_empty(&first).await.unwrap();
    let inserted_second = db.insert_account_if_empty(&second).await.unwrap();

    assert!(inserted_first);
    assert!(!inserted_second);

    let account = db.get_account().await.unwrap().unwrap();
    assert_eq!(account.username, "first");
}

#[tokio::test]
async fn test_patch_account_profile_noop_returns_success() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new_string(),
        username: "patch-user".to_string(),
        display_name: Some("Patch User".to_string()),
        note: Some("original note".to_string()),
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "private_key".to_string(),
        public_key_pem: "public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.upsert_account(&account).await.unwrap();

    let updated = db
        .patch_account_profile(account.id.as_str(), None, None, Utc::now())
        .await
        .unwrap();
    assert!(updated);

    let stored = db.get_account().await.unwrap().unwrap();
    assert_eq!(stored.display_name, Some("Patch User".to_string()));
    assert_eq!(stored.note, Some("original note".to_string()));
}

#[tokio::test]
async fn test_patch_account_credentials_if_matches_updates_profile_and_media_keys() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new_string(),
        username: "credential-user".to_string(),
        display_name: Some("Before".to_string()),
        note: Some("before-note".to_string()),
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: Some("media/old-avatar.webp".to_string()),
        header_s3_key: Some("media/old-header.webp".to_string()),
        private_key_pem: "private_key".to_string(),
        public_key_pem: "public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.upsert_account(&account).await.unwrap();

    let updated = db
        .patch_account_credentials_if_matches(&AccountCredentialsPatch {
            account_id: account.id.clone(),
            expected_current_avatar_s3_key: Some("media/old-avatar.webp".to_string()),
            expected_current_header_s3_key: Some("media/old-header.webp".to_string()),
            avatar_s3_key: Some("media/new-avatar.webp".to_string()),
            header_s3_key: Some("media/new-header.webp".to_string()),
            display_name: Some(Some("After".to_string())),
            note: Some(None),
            profile_fields_json: None,
            locked: None,
            bot: None,
            discoverable: None,
            indexable: None,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    assert!(updated);

    let stored = db.get_account().await.unwrap().unwrap();
    assert_eq!(stored.display_name, Some("After".to_string()));
    assert_eq!(stored.note, None);
    assert_eq!(
        stored.avatar_s3_key,
        Some("media/new-avatar.webp".to_string())
    );
    assert_eq!(
        stored.header_s3_key,
        Some("media/new-header.webp".to_string())
    );
}

#[tokio::test]
async fn test_patch_account_credentials_if_matches_rejects_mismatched_expected_keys() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new_string(),
        username: "credential-user".to_string(),
        display_name: Some("Before".to_string()),
        note: Some("before-note".to_string()),
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: Some("media/old-avatar.webp".to_string()),
        header_s3_key: Some("media/old-header.webp".to_string()),
        private_key_pem: "private_key".to_string(),
        public_key_pem: "public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.upsert_account(&account).await.unwrap();

    let updated = db
        .patch_account_credentials_if_matches(&AccountCredentialsPatch {
            account_id: account.id.clone(),
            expected_current_avatar_s3_key: Some("media/unexpected-avatar.webp".to_string()),
            expected_current_header_s3_key: Some("media/old-header.webp".to_string()),
            avatar_s3_key: Some("media/new-avatar.webp".to_string()),
            header_s3_key: Some("media/new-header.webp".to_string()),
            display_name: Some(Some("After".to_string())),
            note: Some(Some("after-note".to_string())),
            profile_fields_json: None,
            locked: None,
            bot: None,
            discoverable: None,
            indexable: None,
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    assert!(!updated);

    let stored = db.get_account().await.unwrap().unwrap();
    assert_eq!(stored.display_name, Some("Before".to_string()));
    assert_eq!(stored.note, Some("before-note".to_string()));
    assert_eq!(
        stored.avatar_s3_key,
        Some("media/old-avatar.webp".to_string())
    );
    assert_eq!(
        stored.header_s3_key,
        Some("media/old-header.webp".to_string())
    );
}

#[tokio::test]
async fn test_patch_account_migration_updates_alias_and_move_target() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new_string(),
        username: "migration-user".to_string(),
        display_name: Some("Migration User".to_string()),
        note: None,
        profile_fields_json: None,
        locked: false,
        bot: false,
        discoverable: true,
        indexable: true,
        also_known_as: None,
        moved_to_uri: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "private_key".to_string(),
        public_key_pem: "public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.upsert_account(&account).await.unwrap();

    let updated = db
        .patch_account_migration(
            &account.id,
            Some(Some("https://old.example/users/migration-user")),
            Some(Some("https://new.example/users/migration-user")),
            Utc::now(),
        )
        .await
        .unwrap();
    assert!(updated);

    let stored = db.get_account().await.unwrap().unwrap();
    assert_eq!(
        stored.also_known_as.as_deref(),
        Some("https://old.example/users/migration-user")
    );
    assert_eq!(
        stored.moved_to_uri.as_deref(),
        Some("https://new.example/users/migration-user")
    );
}

#[tokio::test]
async fn test_status_crud() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/123".to_string(),
        content: "<p>Hello, world!</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    // Insert status
    db.insert_status(&status).await.unwrap();

    // Get by ID
    let retrieved = db.get_status(&status.id).await.unwrap();
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap().content, "<p>Hello, world!</p>");

    // Get by URI
    let retrieved = db.get_status_by_uri(&status.uri).await.unwrap();
    assert!(retrieved.is_some());

    // Get local statuses
    let statuses = db.get_local_statuses(10, None).await.unwrap();
    assert_eq!(statuses.len(), 1);

    // Delete status
    db.delete_status(&status.id).await.unwrap();
    let retrieved = db.get_status(&status.id).await.unwrap();
    assert!(retrieved.is_none());
}

#[tokio::test]
async fn test_follow_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "user@example.com".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/123".to_string(),
        created_at: Utc::now(),
    };

    // Insert follow
    db.insert_follow(&follow).await.unwrap();

    // Get all follow addresses
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses.len(), 1);
    assert_eq!(addresses[0], "user@example.com");

    // Delete follow
    db.delete_follow("user@example.com", None).await.unwrap();
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses.len(), 0);
}

#[tokio::test]
async fn test_count_statuses_by_account_address_counts_remote_statuses_case_insensitively() {
    let (db, _temp_dir) = create_test_db().await;

    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://local.test/statuses/1".to_string(),
        content: "<p>local</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&local_status).await.unwrap();

    let remote_status_1 = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/1".to_string(),
        content: "<p>remote-1</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "alice@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Reposted,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    db.insert_status(&remote_status_1).await.unwrap();

    let remote_status_2 = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/alice/statuses/2".to_string(),
        content: "<p>remote-2</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "ALICE@REMOTE.EXAMPLE".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Favourited,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    db.insert_status(&remote_status_2).await.unwrap();

    let other_remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/users/bob/statuses/1".to_string(),
        content: "<p>remote-3</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "bob@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Bookmarked,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    db.insert_status(&other_remote_status).await.unwrap();

    let count = db
        .count_statuses_by_account_address("alice@remote.example")
        .await
        .unwrap();
    assert_eq!(count, 2);
}

#[tokio::test]
async fn test_insert_follow_if_absent_deduplicates_default_port_variants() {
    let (db, _temp_dir) = create_test_db().await;

    let first = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/default-port-first".to_string(),
        created_at: Utc::now(),
    };
    let second = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/default-port-second".to_string(),
        created_at: Utc::now(),
    };

    let inserted_first = db.insert_follow_if_absent(&first, Some(443)).await.unwrap();
    let inserted_second = db
        .insert_follow_if_absent(&second, Some(443))
        .await
        .unwrap();

    assert!(inserted_first);
    assert!(!inserted_second);
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses, vec!["alice@remote.example:443".to_string()]);
}

#[tokio::test]
async fn test_insert_follow_if_absent_is_atomic_for_equivalent_targets() {
    let (db, _temp_dir) = create_test_db().await;
    let db = Arc::new(db);
    let barrier = Arc::new(Barrier::new(2));

    let db1 = db.clone();
    let barrier1 = barrier.clone();
    let task1 = tokio::spawn(async move {
        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example:443".to_string(),
            actor_uri: None,
            uri: "https://example.com/follows/atomic-1".to_string(),
            created_at: Utc::now(),
        };
        barrier1.wait().await;
        db1.insert_follow_if_absent(&follow, Some(443))
            .await
            .unwrap()
    });

    let db2 = db.clone();
    let barrier2 = barrier.clone();
    let task2 = tokio::spawn(async move {
        let follow = Follow {
            id: EntityId::new_string(),
            target_address: "alice@remote.example".to_string(),
            actor_uri: None,
            uri: "https://example.com/follows/atomic-2".to_string(),
            created_at: Utc::now(),
        };
        barrier2.wait().await;
        db2.insert_follow_if_absent(&follow, Some(443))
            .await
            .unwrap()
    });

    let inserted1 = task1.await.unwrap();
    let inserted2 = task2.await.unwrap();
    assert_ne!(inserted1, inserted2);

    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses.len(), 1);
}

#[tokio::test]
async fn test_follower_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let follower = Follower {
        id: EntityId::new_string(),
        follower_address: "follower@example.com".to_string(),
        actor_uri: None,
        inbox_uri: "https://example.com/inbox".to_string(),
        uri: "https://example.com/follows/456".to_string(),
        created_at: Utc::now(),
    };

    // Insert follower
    db.insert_follower(&follower).await.unwrap();

    // Get all follower addresses
    let addresses = db.get_all_follower_addresses().await.unwrap();
    assert_eq!(addresses.len(), 1);
    assert_eq!(addresses[0], "follower@example.com");

    // Get follower inboxes
    let inboxes = db.get_follower_inboxes().await.unwrap();
    assert_eq!(inboxes.len(), 1);
    assert_eq!(inboxes[0], "https://example.com/inbox");

    // Delete follower
    db.delete_follower("follower@example.com", None)
        .await
        .unwrap();
    let addresses = db.get_all_follower_addresses().await.unwrap();
    assert_eq!(addresses.len(), 0);
}

#[tokio::test]
async fn test_delete_follower_matches_missing_default_https_port() {
    let (db, _temp_dir) = create_test_db().await;

    let follower = Follower {
        id: EntityId::new_string(),
        follower_address: "bob@remote.example:443".to_string(),
        actor_uri: None,
        inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
        uri: "https://remote.example/follows/default-port".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follower(&follower).await.unwrap();

    db.delete_follower("bob@remote.example", Some(443))
        .await
        .unwrap();
    let addresses = db.get_all_follower_addresses().await.unwrap();
    assert!(addresses.is_empty());
}

#[tokio::test]
async fn test_delete_follower_by_address_and_uri_matches_default_https_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    let follower = Follower {
        id: EntityId::new_string(),
        follower_address: "bob@remote.example".to_string(),
        actor_uri: None,
        inbox_uri: "https://remote.example/users/bob/inbox".to_string(),
        uri: "https://remote.example/follows/default-port-uri".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follower(&follower).await.unwrap();

    let removed = db
        .delete_follower_by_address_and_uri(
            "bob@remote.example:443",
            "https://remote.example/follows/default-port-uri",
            Some(443),
        )
        .await
        .unwrap();
    assert!(removed);

    let addresses = db.get_all_follower_addresses().await.unwrap();
    assert!(addresses.is_empty());
}

#[tokio::test]
async fn test_delete_follow_matches_missing_default_https_port() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/default-port".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    db.delete_follow("alice@remote.example", Some(443))
        .await
        .unwrap();
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert!(addresses.is_empty());
}

#[tokio::test]
async fn test_delete_follow_matches_explicit_default_https_port() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/no-port".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    db.delete_follow("alice@remote.example:443", Some(443))
        .await
        .unwrap();
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert!(addresses.is_empty());
}

#[tokio::test]
async fn test_delete_follow_does_not_match_non_default_port() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:80".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/non-default-port".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    db.delete_follow("alice@remote.example", Some(443))
        .await
        .unwrap();
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses, vec!["alice@remote.example:80".to_string()]);
}

#[tokio::test]
async fn test_get_follow_uri_matches_case_insensitively() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "Alice@Remote.EXAMPLE".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/case-insensitive".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    let uri = db
        .get_follow_uri("alice@remote.example", Some(443))
        .await
        .unwrap();
    assert_eq!(
        uri,
        Some("https://example.com/follows/case-insensitive".to_string())
    );
}

#[tokio::test]
async fn test_get_follow_uri_matches_default_https_port_variants() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/default-port-uri".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    let uri = db
        .get_follow_uri("alice@remote.example", Some(443))
        .await
        .unwrap();
    assert_eq!(
        uri,
        Some("https://example.com/follows/default-port-uri".to_string())
    );
}

#[tokio::test]
async fn test_get_follow_uri_does_not_match_non_default_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/no-port-uri".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    let uri = db
        .get_follow_uri("alice@remote.example:80", Some(443))
        .await
        .unwrap();
    assert_eq!(uri, None);
}

#[tokio::test]
async fn test_update_follow_actor_uri_matches_default_https_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/actor-uri-update".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    db.update_follow_actor_uri(
        "alice@remote.example",
        "https://remote.example/@alice",
        Some(443),
    )
    .await
    .unwrap();

    let follows = db.get_all_follows().await.unwrap();
    assert_eq!(follows.len(), 1);
    assert_eq!(
        follows[0].actor_uri.as_deref(),
        Some("https://remote.example/@alice")
    );
}

#[tokio::test]
async fn test_block_account_removes_follow_for_default_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new_string(),
        target_address: "alice@remote.example:443".to_string(),
        actor_uri: None,
        uri: "https://example.com/follows/block-match".to_string(),
        created_at: Utc::now(),
    };
    db.insert_follow(&follow).await.unwrap();

    assert!(
        db.block_account("alice@remote.example", Some(443))
            .await
            .unwrap()
    );

    let follow_addresses = db.get_all_follow_addresses().await.unwrap();
    assert!(follow_addresses.is_empty());
    assert!(
        db.is_account_blocked("alice@remote.example:443", Some(443))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_block_account_returns_false_when_address_variant_already_blocked() {
    let (db, _temp_dir) = create_test_db().await;

    assert!(
        db.block_account("alice@remote.example", Some(443))
            .await
            .unwrap()
    );
    assert!(
        !db.block_account("alice@remote.example:443", Some(443))
            .await
            .unwrap()
    );

    let blocked_accounts = db.get_blocked_accounts(10).await.unwrap();
    assert_eq!(blocked_accounts, vec!["alice@remote.example".to_string()]);
}

#[tokio::test]
async fn test_mute_unmute_matches_default_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    db.mute_account("alice@remote.example:443", true, None, Some(443))
        .await
        .unwrap();
    assert!(
        db.is_account_muted("alice@remote.example", Some(443))
            .await
            .unwrap()
    );

    db.unmute_account("alice@remote.example", Some(443))
        .await
        .unwrap();
    assert!(
        !db.is_account_muted("alice@remote.example:443", Some(443))
            .await
            .unwrap()
    );
}

#[tokio::test]
async fn test_get_muted_account_details_preserves_actor_uri() {
    let (db, _temp_dir) = create_test_db().await;

    db.mute_account_with_actor_uri(
        "alice@remote.example",
        true,
        None,
        Some("https://remote.example/users/alice"),
        Some(443),
    )
    .await
    .unwrap();

    let muted = db.get_muted_account_details(10).await.unwrap();
    assert_eq!(muted.len(), 1);
    assert_eq!(muted[0].0, "alice@remote.example");
    assert_eq!(
        muted[0].1.as_deref(),
        Some("https://remote.example/users/alice")
    );
}

#[tokio::test]
async fn test_notification_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let notification = Notification {
        id: EntityId::new_string(),
        notification_type: NotificationType::Mention,
        origin_account_address: "user@example.com".to_string(),
        status_uri: Some("https://example.com/status/123".to_string()),
        read: false,
        created_at: Utc::now(),
    };

    // Insert notification
    db.insert_notification(&notification).await.unwrap();

    // Get unread notifications
    let notifications = db.get_notifications(10, None, true).await.unwrap();
    assert_eq!(notifications.len(), 1);
    assert_eq!(
        notifications[0].notification_type,
        crate::data::NotificationType::Mention
    );

    // Mark as read
    db.mark_notification_read(&notification.id).await.unwrap();
    let notifications = db.get_notifications(10, None, true).await.unwrap();
    assert_eq!(notifications.len(), 0);
}

#[tokio::test]
async fn test_insert_notification_if_new_deduplicates_by_activity_identity() {
    let (db, _temp_dir) = create_test_db().await;
    let activity_uri = "https://remote.example/activities/update-1";

    let first = Notification {
        id: "notif-identity-1".to_string(),
        notification_type: NotificationType::QuotedUpdate,
        origin_account_address: "alice@remote.example".to_string(),
        status_uri: Some("https://local.example/statuses/quote-1".to_string()),
        read: false,
        created_at: Utc::now(),
    };
    let duplicate = Notification {
        id: "notif-identity-2".to_string(),
        ..first.clone()
    };
    let second_target = Notification {
        id: "notif-identity-3".to_string(),
        status_uri: Some("https://local.example/statuses/quote-2".to_string()),
        ..first.clone()
    };
    let second_type = Notification {
        id: "notif-identity-4".to_string(),
        notification_type: NotificationType::Mention,
        ..first.clone()
    };

    assert!(
        db.insert_notification_if_new(&first, Some(activity_uri))
            .await
            .unwrap()
    );
    assert!(
        !db.insert_notification_if_new(&duplicate, Some(activity_uri))
            .await
            .unwrap()
    );
    assert!(
        db.insert_notification_if_new(&second_target, Some(activity_uri))
            .await
            .unwrap()
    );
    assert!(
        db.insert_notification_if_new(&second_type, Some(activity_uri))
            .await
            .unwrap()
    );

    let notifications = db.get_notifications(10, None, false).await.unwrap();
    assert_eq!(notifications.len(), 3);
}

#[tokio::test]
async fn test_get_notifications_paginates_by_created_at_then_id() {
    let (db, _temp_dir) = create_test_db().await;

    let shared_time = Utc::now();
    let oldest = Notification {
        id: "notif-001".to_string(),
        notification_type: NotificationType::Mention,
        origin_account_address: "user1@example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: shared_time - chrono::Duration::seconds(1),
    };
    let middle = Notification {
        id: "notif-002".to_string(),
        notification_type: NotificationType::Reblog,
        origin_account_address: "user2@example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: shared_time,
    };
    let newest_same_time = Notification {
        id: "notif-003".to_string(),
        notification_type: NotificationType::Favourite,
        origin_account_address: "user3@example.com".to_string(),
        status_uri: None,
        read: false,
        created_at: shared_time,
    };

    db.insert_notification(&oldest).await.unwrap();
    db.insert_notification(&middle).await.unwrap();
    db.insert_notification(&newest_same_time).await.unwrap();

    let first_page = db.get_notifications(2, None, false).await.unwrap();
    let first_page_ids: Vec<&str> = first_page.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(first_page_ids, vec!["notif-003", "notif-002"]);

    let second_page = db
        .get_notifications(2, Some("notif-002"), false)
        .await
        .unwrap();
    let second_page_ids: Vec<&str> = second_page.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(second_page_ids, vec!["notif-001"]);
}

#[tokio::test]
async fn test_favourite_operations() {
    let (db, _temp_dir) = create_test_db().await;

    // Create a status first (required for foreign key)
    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/fav".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let status_id = &status.id;

    // Initially not favourited
    assert!(!db.is_favourited(status_id).await.unwrap());

    // Insert favourite
    db.insert_favourite(status_id).await.unwrap();

    // Now favourited
    assert!(db.is_favourited(status_id).await.unwrap());

    // Get favourited IDs
    let ids = db.get_favourited_status_ids(10).await.unwrap();
    assert_eq!(ids.len(), 1);

    // Delete favourite
    db.delete_favourite(status_id).await.unwrap();
    assert!(!db.is_favourited(status_id).await.unwrap());
}

#[tokio::test]
async fn test_bookmark_operations() {
    let (db, _temp_dir) = create_test_db().await;

    // Create a status first (required for foreign key)
    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/bookmark".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let status_id = &status.id;

    // Initially not bookmarked
    assert!(!db.is_bookmarked(status_id).await.unwrap());

    // Insert bookmark
    db.insert_bookmark(status_id).await.unwrap();

    // Now bookmarked
    assert!(db.is_bookmarked(status_id).await.unwrap());

    // Delete bookmark
    db.delete_bookmark(status_id).await.unwrap();
    assert!(!db.is_bookmarked(status_id).await.unwrap());
}

#[tokio::test]
async fn test_repost_operations() {
    let (db, _temp_dir) = create_test_db().await;

    // Create a status first (required for foreign key)
    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/repost".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let status_id = &status.id;

    // Initially not reposted
    assert!(!db.is_reposted(status_id).await.unwrap());

    // Insert repost
    db.insert_repost(status_id, "https://example.com/activity/repost")
        .await
        .unwrap();

    // Now reposted
    assert!(db.is_reposted(status_id).await.unwrap());

    // Delete repost
    db.delete_repost(status_id).await.unwrap();
    assert!(!db.is_reposted(status_id).await.unwrap());
}

#[tokio::test]
async fn test_status_pin_and_mute_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let root = Status {
        id: "pin-mute-root".to_string(),
        uri: "https://example.com/status/pin-mute-root".to_string(),
        content: "<p>Pin and mute root</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let reply = Status {
        id: "pin-mute-reply".to_string(),
        uri: "https://example.com/status/pin-mute-reply".to_string(),
        content: "<p>Pin and mute reply</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&root).await.unwrap();
    db.insert_status(&reply).await.unwrap();

    let reply_thread_uri = db.resolve_thread_root_uri(&reply).await.unwrap();
    assert_eq!(reply_thread_uri, root.uri);

    assert!(!db.is_status_pinned(&root.id).await.unwrap());
    assert!(!db.is_thread_muted(&root.uri).await.unwrap());

    db.insert_status_pin(&root.id).await.unwrap();
    db.insert_muted_thread(&root.uri).await.unwrap();

    assert!(db.is_status_pinned(&root.id).await.unwrap());
    assert!(db.is_thread_muted(&root.uri).await.unwrap());

    db.delete_status_pin(&root.id).await.unwrap();
    db.delete_muted_thread(&root.uri).await.unwrap();

    assert!(!db.is_status_pinned(&root.id).await.unwrap());
    assert!(!db.is_thread_muted(&root.uri).await.unwrap());
}

#[tokio::test]
async fn test_resolve_thread_root_uri_handles_reply_chains_deeper_than_256() {
    let (db, _temp_dir) = create_test_db().await;

    let root_uri = "https://example.com/status/deep-root".to_string();
    let root = Status {
        id: "deep-root".to_string(),
        uri: root_uri.clone(),
        content: "<p>Root</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&root).await.unwrap();

    let mut parent_uri = root_uri.clone();
    let mut leaf_status: Option<Status> = None;
    for depth in 0..300 {
        let status = Status {
            id: format!("deep-reply-{depth}"),
            uri: format!("https://example.com/status/deep-reply-{depth}"),
            content: format!("<p>Reply {depth}</p>"),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: Some(parent_uri.clone()),
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        db.insert_status(&status).await.unwrap();
        parent_uri = status.uri.clone();
        leaf_status = Some(status);
    }

    let leaf_status = leaf_status.expect("leaf status must exist");
    let resolved = db.resolve_thread_root_uri(&leaf_status).await.unwrap();
    assert_eq!(resolved, root_uri);
}

#[tokio::test]
async fn test_status_reply_lookup_and_edit_history_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let parent = Status {
        id: "parent-status".to_string(),
        uri: "https://example.com/status/parent".to_string(),
        content: "<p>Parent</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let child_a = Status {
        id: "child-a".to_string(),
        uri: "https://example.com/status/child-a".to_string(),
        content: "<p>Child A</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(parent.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let child_b = Status {
        id: "child-b".to_string(),
        uri: "https://example.com/status/child-b".to_string(),
        content: "<p>Child B</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(parent.uri.clone()),
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&parent).await.unwrap();
    db.insert_status(&child_a).await.unwrap();
    db.insert_status(&child_b).await.unwrap();

    let replies = db.get_status_replies(&parent.uri).await.unwrap();
    assert_eq!(replies.len(), 2);
    assert_eq!(replies[0].id, "child-a");
    assert_eq!(replies[1].id, "child-b");
    let limited_replies = db.get_status_replies_limited(&parent.uri, 1).await.unwrap();
    assert_eq!(limited_replies.len(), 1);
    assert_eq!(limited_replies[0].id, "child-a");

    db.insert_status_edit(&parent.id, "<p>Parent v1</p>", None)
        .await
        .unwrap();
    db.insert_status_edit(&parent.id, "<p>Parent v2</p>", Some("cw"))
        .await
        .unwrap();
    let edits = db.get_status_edits(&parent.id, 10).await.unwrap();
    assert_eq!(edits.len(), 2);
    assert!(
        edits
            .iter()
            .any(|(_, content, _, _)| content == "<p>Parent v1</p>")
    );
    assert!(
        edits
            .iter()
            .any(|(_, content, _, _)| content == "<p>Parent v2</p>")
    );
}

#[tokio::test]
async fn test_bookmarked_statuses_order_and_cursor_by_bookmark_time() {
    let (db, _temp_dir) = create_test_db().await;

    for id in ["100", "200", "300"] {
        let status = Status {
            id: id.to_string(),
            uri: format!("https://example.com/status/{}", id),
            content: "<p>Test</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        db.insert_status(&status).await.unwrap();
    }

    // Intentionally make bookmark order different from status ID order.
    db.insert_bookmark("300").await.unwrap();
    db.insert_bookmark("100").await.unwrap();
    db.insert_bookmark("200").await.unwrap();
    db.set_bookmark_created_at_for_test("300", "2024-01-01 00:00:01")
        .await
        .unwrap();
    db.set_bookmark_created_at_for_test("100", "2024-01-01 00:00:02")
        .await
        .unwrap();
    db.set_bookmark_created_at_for_test("200", "2024-01-01 00:00:03")
        .await
        .unwrap();

    let all = db.get_bookmarked_statuses(10, None).await.unwrap();
    let all_ids: Vec<_> = all.into_iter().map(|s| s.id).collect();
    assert_eq!(all_ids, vec!["200", "100", "300"]);

    let next_page = db.get_bookmarked_statuses(10, Some("100")).await.unwrap();
    let next_ids: Vec<_> = next_page.into_iter().map(|s| s.id).collect();
    assert_eq!(next_ids, vec!["300"]);
}

#[tokio::test]
async fn test_favourited_statuses_order_and_cursor_by_favourite_time() {
    let (db, _temp_dir) = create_test_db().await;

    for id in ["400", "500", "600"] {
        let status = Status {
            id: id.to_string(),
            uri: format!("https://example.com/status/{}", id),
            content: "<p>Test</p>".to_string(),
            content_warning: None,
            visibility: crate::data::StatusVisibility::Public,
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            quote_of_uri: None,
            persisted_reason: PersistedReason::Own,
            created_at: Utc::now(),
            fetched_at: None,
        };
        db.insert_status(&status).await.unwrap();
    }

    // Intentionally make favourite order different from status ID order.
    db.insert_favourite("600").await.unwrap();
    db.insert_favourite("400").await.unwrap();
    db.insert_favourite("500").await.unwrap();
    db.set_favourite_created_at_for_test("600", "2024-01-01 00:00:01")
        .await
        .unwrap();
    db.set_favourite_created_at_for_test("400", "2024-01-01 00:00:02")
        .await
        .unwrap();
    db.set_favourite_created_at_for_test("500", "2024-01-01 00:00:03")
        .await
        .unwrap();

    let all = db.get_favourited_statuses(10, None).await.unwrap();
    let all_ids: Vec<_> = all.into_iter().map(|s| s.id).collect();
    assert_eq!(all_ids, vec!["500", "400", "600"]);

    let next_page = db.get_favourited_statuses(10, Some("400")).await.unwrap();
    let next_ids: Vec<_> = next_page.into_iter().map(|s| s.id).collect();
    assert_eq!(next_ids, vec!["600"]);
}

#[tokio::test]
async fn test_domain_block_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let domain = "spam.example.com";

    // Initially not blocked
    assert!(!db.is_domain_blocked(domain).await.unwrap());

    // Block domain
    db.block_domain(domain).await.unwrap();
    // Duplicate requests should be idempotent
    db.block_domain(domain).await.unwrap();

    // Now blocked
    assert!(db.is_domain_blocked(domain).await.unwrap());

    // Get all blocked domains
    let domains = db.get_blocked_domains().await.unwrap();
    assert_eq!(domains.len(), 1);
    assert_eq!(domains[0], domain);

    // Unblock domain
    db.unblock_domain(domain).await.unwrap();
    assert!(!db.is_domain_blocked(domain).await.unwrap());
}

#[tokio::test]
async fn test_domain_block_attributes_persist_and_update() {
    let (db, _temp_dir) = create_test_db().await;

    let created = db
        .upsert_domain_block(
            "remote.example",
            "silence",
            false,
            false,
            Some("private note"),
            Some("public note"),
            true,
        )
        .await
        .unwrap();

    assert_eq!(created.domain, "remote.example");
    assert_eq!(created.severity, "silence");
    assert!(!created.reject_media);
    assert!(!created.reject_reports);
    assert_eq!(created.private_comment.as_deref(), Some("private note"));
    assert_eq!(created.public_comment.as_deref(), Some("public note"));
    assert!(created.obfuscate);

    let updated = db
        .upsert_domain_block(
            "remote.example",
            "suspend",
            true,
            true,
            Some("updated private"),
            Some("updated public"),
            false,
        )
        .await
        .unwrap();

    assert_eq!(updated.id, created.id);
    assert_eq!(updated.severity, "suspend");
    assert!(updated.reject_media);
    assert!(updated.reject_reports);
    assert_eq!(updated.private_comment.as_deref(), Some("updated private"));
    assert_eq!(updated.public_comment.as_deref(), Some("updated public"));
    assert!(!updated.obfuscate);

    let fetched = db
        .get_domain_block_by_id(&created.id)
        .await
        .unwrap()
        .expect("persisted domain block");
    assert_eq!(fetched.id, created.id);
    assert_eq!(fetched.severity, "suspend");
    assert!(fetched.reject_media);
    assert!(fetched.reject_reports);
    assert_eq!(fetched.private_comment.as_deref(), Some("updated private"));
    assert_eq!(fetched.public_comment.as_deref(), Some("updated public"));
    assert!(!fetched.obfuscate);
}

#[tokio::test]
async fn test_settings_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let key = "test_key";
    let value = "test_value";

    // Initially no value
    assert!(db.get_setting(key).await.unwrap().is_none());

    // Set value
    db.set_setting(key, value).await.unwrap();

    // Get value
    let retrieved = db.get_setting(key).await.unwrap();
    assert_eq!(retrieved, Some(value.to_string()));

    // Update value
    db.set_setting(key, "new_value").await.unwrap();
    let retrieved = db.get_setting(key).await.unwrap();
    assert_eq!(retrieved, Some("new_value".to_string()));
}

#[tokio::test]
async fn test_list_batch_add_and_remove_accounts() {
    let (db, _temp_dir) = create_test_db().await;

    let list_id = db.create_list("Test List", "list").await.unwrap();
    let add_accounts = vec![
        "alice@example.com".to_string(),
        "bob@example.com".to_string(),
        "carol@example.com".to_string(),
    ];
    db.add_accounts_to_list(&list_id, &add_accounts)
        .await
        .unwrap();

    let mut stored = db.get_list_accounts(&list_id).await.unwrap();
    stored.sort();
    let mut expected = add_accounts.clone();
    expected.sort();
    assert_eq!(stored, expected);

    let remove_accounts = vec![
        "alice@example.com".to_string(),
        "carol@example.com".to_string(),
    ];
    db.remove_accounts_from_list(&list_id, &remove_accounts)
        .await
        .unwrap();

    let remaining = db.get_list_accounts(&list_id).await.unwrap();
    assert_eq!(remaining, vec!["bob@example.com".to_string()]);
}

#[tokio::test]
async fn test_insert_status_indexes_hashtags_and_tag_timeline_query() {
    let (db, _temp_dir) = create_test_db().await;
    let base_time = Utc::now();

    let old_public = Status {
        id: "200".to_string(),
        uri: "https://example.com/status/tag-old".to_string(),
        content: "<p>Old #Rust post</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time,
        fetched_at: None,
    };
    let new_public = Status {
        id: "300".to_string(),
        uri: "https://example.com/status/tag-new".to_string(),
        content: "<p>New #rust and #RustLang post</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time + chrono::Duration::seconds(1),
        fetched_at: None,
    };
    let private_status = Status {
        id: "400".to_string(),
        uri: "https://example.com/status/tag-private".to_string(),
        content: "<p>Private #RUST post</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Private,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time + chrono::Duration::seconds(2),
        fetched_at: None,
    };

    db.insert_status(&old_public).await.unwrap();
    db.insert_status(&new_public).await.unwrap();
    db.insert_status(&private_status).await.unwrap();

    let rust_statuses = db
        .get_statuses_by_hashtag_in_window("rust", 10, None, None)
        .await
        .unwrap();
    let rust_ids: Vec<String> = rust_statuses.into_iter().map(|status| status.id).collect();
    assert_eq!(rust_ids, vec![new_public.id.clone(), old_public.id.clone()]);
    assert!(!rust_ids.contains(&private_status.id));

    let rustlang_statuses = db
        .get_statuses_by_hashtag_in_window("RUSTLANG", 10, None, None)
        .await
        .unwrap();
    assert_eq!(rustlang_statuses.len(), 1);
    assert_eq!(rustlang_statuses[0].id, new_public.id);

    let hashtags = db.search_hashtags("rust", 10).await.unwrap();
    let rust = hashtags
        .iter()
        .find(|(name, _, _)| name.eq_ignore_ascii_case("rust"))
        .expect("missing rust hashtag");
    assert_eq!(rust.1, 3);
}

#[tokio::test]
async fn test_insert_status_indexes_markup_wrapped_hashtags() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: "350".to_string(),
        uri: "https://example.com/status/tag-markup".to_string(),
        content: "<p>#<span>Rust</span> and #<a href=\"https://example.com/tags/rustlang\">RustLang</a></p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let rust = db
        .get_statuses_by_hashtag_in_window("rust", 10, None, None)
        .await
        .unwrap();
    let rustlang = db
        .get_statuses_by_hashtag_in_window("rustlang", 10, None, None)
        .await
        .unwrap();
    assert_eq!(rust.len(), 1);
    assert_eq!(rustlang.len(), 1);
    assert_eq!(rust[0].id, status.id);
    assert_eq!(rustlang[0].id, status.id);
}

#[tokio::test]
async fn test_database_connect_backfills_missing_status_hashtag_rows() {
    let (db, temp_dir) = create_test_db().await;

    let status = Status {
        id: "500".to_string(),
        uri: "https://example.com/status/backfill-hashtag".to_string(),
        content: "<p>Legacy #Rust post</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let raw_pool = SqlitePool::connect(&test_db_connection_string(&temp_dir))
        .await
        .unwrap();
    sqlx::query("DELETE FROM status_hashtags WHERE status_id = ?")
        .bind(&status.id)
        .execute(&raw_pool)
        .await
        .unwrap();
    drop(raw_pool);
    drop(db);

    let reopened = Database::connect(&temp_dir.path().join("test.db"))
        .await
        .expect("reopen should succeed with backfill");
    let statuses = reopened
        .get_statuses_by_hashtag_in_window("rust", 10, None, None)
        .await
        .unwrap();
    let ids: Vec<String> = statuses.into_iter().map(|entry| entry.id).collect();
    assert!(ids.contains(&status.id));
}

#[tokio::test]
async fn test_tag_timeline_query_uses_id_aligned_cursors() {
    let (db, _temp_dir) = create_test_db().await;
    let base_time = Utc::now();

    let newest_by_id = Status {
        id: "300".to_string(),
        uri: "https://example.com/status/tag-300".to_string(),
        content: "<p>#Rust id300</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time,
        fetched_at: None,
    };
    let middle_by_id = Status {
        id: "200".to_string(),
        uri: "https://example.com/status/tag-200".to_string(),
        content: "<p>#Rust id200</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time + chrono::Duration::seconds(10),
        fetched_at: None,
    };
    let oldest_by_id = Status {
        id: "100".to_string(),
        uri: "https://example.com/status/tag-100".to_string(),
        content: "<p>#Rust id100</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time + chrono::Duration::seconds(20),
        fetched_at: None,
    };

    db.insert_status(&newest_by_id).await.unwrap();
    db.insert_status(&middle_by_id).await.unwrap();
    db.insert_status(&oldest_by_id).await.unwrap();

    let first_page = db
        .get_statuses_by_hashtag_in_window("rust", 2, None, None)
        .await
        .unwrap();
    let first_ids: Vec<String> = first_page.into_iter().map(|status| status.id).collect();
    assert_eq!(first_ids, vec!["100".to_string(), "200".to_string()]);

    let older_page = db
        .get_statuses_by_hashtag_in_window("rust", 2, Some("200"), None)
        .await
        .unwrap();
    let older_ids: Vec<String> = older_page.into_iter().map(|status| status.id).collect();
    assert_eq!(older_ids, vec!["300".to_string()]);

    let newer_page = db
        .get_statuses_by_hashtag_in_window("rust", 2, None, Some("200"))
        .await
        .unwrap();
    let newer_ids: Vec<String> = newer_page.into_iter().map(|status| status.id).collect();
    assert_eq!(newer_ids, vec!["100".to_string()]);
}

#[tokio::test]
async fn test_update_status_refreshes_hashtag_index() {
    let (db, _temp_dir) = create_test_db().await;
    let mut status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/tag-update".to_string(),
        content: "<p>Initial #OldTag</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let old_before = db
        .get_statuses_by_hashtag_in_window("oldtag", 10, None, None)
        .await
        .unwrap();
    assert_eq!(old_before.len(), 1);

    status.content = "<p>Updated #newtag</p>".to_string();
    db.update_status(&status).await.unwrap();

    let old_after = db
        .get_statuses_by_hashtag_in_window("oldtag", 10, None, None)
        .await
        .unwrap();
    assert!(old_after.is_empty());

    let new_after = db
        .get_statuses_by_hashtag_in_window("newtag", 10, None, None)
        .await
        .unwrap();
    assert_eq!(new_after.len(), 1);
    assert_eq!(new_after[0].id, status.id);
}

#[tokio::test]
async fn test_list_timeline_query_matches_local_and_remote_accounts() {
    let (db, _temp_dir) = create_test_db().await;
    let list_id = db.create_list("List timeline", "list").await.unwrap();
    let local_address = "testuser@test.example.com".to_string();
    let local_account_id = "local-account-id".to_string();
    let remote_address = "alice@example.com".to_string();
    db.add_accounts_to_list(&list_id, &[local_address.clone(), remote_address.clone()])
        .await
        .unwrap();

    let base_time = Utc::now();
    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/list-local".to_string(),
        content: "<p>Local status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: base_time + chrono::Duration::seconds(1),
        fetched_at: None,
    };
    let remote_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-remote".to_string(),
        content: "<p>Remote status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: remote_address.clone(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Favourited,
        created_at: base_time + chrono::Duration::seconds(2),
        fetched_at: None,
    };
    let unrelated_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-unrelated".to_string(),
        content: "<p>Unrelated status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "bob@example.com".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Favourited,
        created_at: base_time,
        fetched_at: None,
    };

    db.insert_status(&local_status).await.unwrap();
    db.insert_status(&remote_status).await.unwrap();
    db.insert_status(&unrelated_status).await.unwrap();

    let statuses = db
        .get_list_timeline_statuses_in_window(&ListTimelineQuery {
            list_id: list_id.clone(),
            local_account_address: local_address.clone(),
            local_account_id: local_account_id.clone(),
            default_port: Some(443),
            limit: 10,
            max_id: None,
            min_id: None,
        })
        .await
        .unwrap();
    let ids: Vec<String> = statuses.into_iter().map(|status| status.id).collect();

    assert!(ids.contains(&local_status.id));
    assert!(ids.contains(&remote_status.id));
    assert!(!ids.contains(&unrelated_status.id));
}

#[tokio::test]
async fn test_list_timeline_query_matches_local_account_id_entries() {
    let (db, _temp_dir) = create_test_db().await;
    let list_id = db.create_list("List timeline by id", "list").await.unwrap();
    let local_address = "testuser@test.example.com".to_string();
    let local_account_id = "01HLOCALACCOUNTID".to_string();
    db.add_accounts_to_list(&list_id, std::slice::from_ref(&local_account_id))
        .await
        .unwrap();

    let local_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/list-local-by-id".to_string(),
        content: "<p>Local status by id list member</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&local_status).await.unwrap();

    let statuses = db
        .get_list_timeline_statuses_in_window(&ListTimelineQuery {
            list_id: list_id.clone(),
            local_account_address: local_address.clone(),
            local_account_id: local_account_id.clone(),
            default_port: Some(443),
            limit: 10,
            max_id: None,
            min_id: None,
        })
        .await
        .unwrap();
    let ids: Vec<String> = statuses.into_iter().map(|status| status.id).collect();
    assert!(ids.contains(&local_status.id));
}

#[tokio::test]
async fn test_list_timeline_query_matches_default_port_equivalent_remote_addresses() {
    let (db, _temp_dir) = create_test_db().await;
    let list_id = db
        .create_list("List timeline default-port", "list")
        .await
        .unwrap();
    let local_address = "testuser@test.example.com".to_string();
    let local_account_id = "01HLOCALACCOUNTID".to_string();
    db.add_accounts_to_list(&list_id, &[String::from("alice@remote.example")])
        .await
        .unwrap();

    let matching_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-default-port-match".to_string(),
        content: "<p>Remote status from default-port equivalent address</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "alice@remote.example:443".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Favourited,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let unrelated_status = Status {
        id: EntityId::new_string(),
        uri: "https://remote.example/status/list-default-port-unrelated".to_string(),
        content: "<p>Unrelated remote status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "bob@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Favourited,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&matching_status).await.unwrap();
    db.insert_status(&unrelated_status).await.unwrap();

    let statuses = db
        .get_list_timeline_statuses_in_window(&ListTimelineQuery {
            list_id: list_id.clone(),
            local_account_address: local_address.clone(),
            local_account_id: local_account_id.clone(),
            default_port: Some(443),
            limit: 10,
            max_id: None,
            min_id: None,
        })
        .await
        .unwrap();
    let ids: Vec<String> = statuses.into_iter().map(|status| status.id).collect();
    assert!(ids.contains(&matching_status.id));
    assert!(!ids.contains(&unrelated_status.id));
}

#[tokio::test]
async fn test_insert_status_with_media_attaches_all_media_atomically() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/with-media".to_string(),
        content: "<p>Status with media</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    let media_ids = vec![EntityId::new_string(), EntityId::new_string()];
    for media_id in &media_ids {
        db.insert_media(&MediaAttachment {
            id: media_id.clone(),
            status_id: None,
            s3_key: format!("media/{}.png", media_id),
            thumbnail_s3_key: None,
            content_type: "image/png".to_string(),
            file_size: 100,
            description: None,
            blurhash: None,
            width: Some(1),
            height: Some(1),
            focus_x: None,
            focus_y: None,
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    }

    db.insert_status_with_media(&status, &media_ids)
        .await
        .unwrap();

    let stored = db.get_status(&status.id).await.unwrap();
    assert!(stored.is_some());

    for media_id in &media_ids {
        let media = db.get_media(media_id).await.unwrap().unwrap();
        assert_eq!(media.status_id, Some(status.id.clone()));
    }
}

#[tokio::test]
async fn test_insert_status_with_media_rolls_back_when_media_missing() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/missing-media".to_string(),
        content: "<p>Status with missing media</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    let result = db
        .insert_status_with_media(&status, &[EntityId::new_string()])
        .await;
    assert!(result.is_err());
    assert!(db.get_status(&status.id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_insert_status_with_media_rolls_back_when_media_already_attached() {
    let (db, _temp_dir) = create_test_db().await;

    let existing_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/existing".to_string(),
        content: "<p>Existing status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&existing_status).await.unwrap();

    let media_id = EntityId::new_string();
    db.insert_media(&MediaAttachment {
        id: media_id.clone(),
        status_id: Some(existing_status.id.clone()),
        s3_key: format!("media/{}.png", media_id),
        thumbnail_s3_key: None,
        content_type: "image/png".to_string(),
        file_size: 100,
        description: None,
        blurhash: None,
        width: Some(1),
        height: Some(1),
        focus_x: None,
        focus_y: None,
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    let new_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/new".to_string(),
        content: "<p>New status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };

    let result = db
        .insert_status_with_media(&new_status, std::slice::from_ref(&media_id))
        .await;
    assert!(result.is_err());
    assert!(db.get_status(&new_status.id).await.unwrap().is_none());

    let media = db.get_media(&media_id).await.unwrap().unwrap();
    assert_eq!(media.status_id, Some(existing_status.id.clone()));
}

#[tokio::test]
async fn test_insert_status_with_media_and_poll_persists_poll_atomically() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/with-poll".to_string(),
        content: "<p>Status with poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let poll_options = vec!["yes".to_string(), "no".to_string()];

    db.insert_status_with_media_and_poll(&status, &[], Some((&poll_options, 600, false)))
        .await
        .unwrap();

    let stored = db.get_status(&status.id).await.unwrap();
    assert!(stored.is_some());
    let poll = db.get_poll_by_status_id(&status.id).await.unwrap();
    assert!(poll.is_some());
    let poll_id = poll.unwrap().0;
    let options = db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].1, "yes");
    assert_eq!(options[1].1, "no");
}

#[tokio::test]
async fn test_insert_status_with_media_and_poll_rolls_back_when_media_missing() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/with-poll-missing-media".to_string(),
        content: "<p>Status with poll and missing media</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let poll_options = vec!["a".to_string(), "b".to_string()];

    let result = db
        .insert_status_with_media_and_poll(
            &status,
            &[EntityId::new_string()],
            Some((&poll_options, 600, false)),
        )
        .await;
    assert!(result.is_err());
    assert!(db.get_status(&status.id).await.unwrap().is_none());
    assert!(
        db.get_poll_by_status_id(&status.id)
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn test_vote_in_poll_rejects_duplicate_option_and_rolls_back_counts() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-duplicate-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .create_poll(&status.id, &["a".to_string(), "b".to_string()], 600, true)
        .await
        .unwrap();
    let options = db.get_poll_options(&poll_id).await.unwrap();
    let option_id = options[0].0.clone();

    let result = db
        .vote_in_poll(
            &poll_id,
            "alice@remote.example",
            &[option_id.clone(), option_id],
        )
        .await;
    assert!(result.is_err());

    let options_after = db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options_after[0].2, 0);
    assert_eq!(options_after[1].2, 0);
    let poll_after = db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll_after.4, 0);
    assert_eq!(poll_after.5, 0);
}

#[tokio::test]
async fn test_vote_in_poll_rejects_option_from_other_poll() {
    let (db, _temp_dir) = create_test_db().await;

    let status_1 = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-1".to_string(),
        content: "<p>Poll 1</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let status_2 = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-2".to_string(),
        content: "<p>Poll 2</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status_1).await.unwrap();
    db.insert_status(&status_2).await.unwrap();

    let poll_1 = db
        .create_poll(&status_1.id, &["a".to_string(), "b".to_string()], 600, true)
        .await
        .unwrap();
    let poll_2 = db
        .create_poll(&status_2.id, &["x".to_string(), "y".to_string()], 600, true)
        .await
        .unwrap();
    let poll_2_options = db.get_poll_options(&poll_2).await.unwrap();
    let foreign_option_id = poll_2_options[0].0.clone();

    let result = db
        .vote_in_poll(&poll_1, "bob@remote.example", &[foreign_option_id])
        .await;
    assert!(result.is_err());

    let poll_1_after = db.get_poll(&poll_1).await.unwrap().unwrap();
    let poll_2_after = db.get_poll(&poll_2).await.unwrap().unwrap();
    assert_eq!(poll_1_after.4, 0);
    assert_eq!(poll_1_after.5, 0);
    assert_eq!(poll_2_after.4, 0);
    assert_eq!(poll_2_after.5, 0);
}

#[tokio::test]
async fn test_vote_in_poll_rejects_second_ballot_for_multiple_poll() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-multiple-second-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .create_poll(&status.id, &["a".to_string(), "b".to_string()], 600, true)
        .await
        .unwrap();
    let options = db.get_poll_options(&poll_id).await.unwrap();
    let first_option_id = options[0].0.clone();
    let second_option_id = options[1].0.clone();

    db.vote_in_poll(
        &poll_id,
        "alice@remote.example",
        std::slice::from_ref(&first_option_id),
    )
    .await
    .unwrap();

    let result = db
        .vote_in_poll(
            &poll_id,
            "alice@remote.example",
            std::slice::from_ref(&second_option_id),
        )
        .await;
    assert!(result.is_err());

    let options_after = db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options_after[0].2, 1);
    assert_eq!(options_after[1].2, 0);
    let poll_after = db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll_after.4, 1);
    assert_eq!(poll_after.5, 1);
}

#[tokio::test]
async fn test_get_poll_marks_immediately_expired_poll_as_expired() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-immediate-expire".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .create_poll(&status.id, &["yes".to_string(), "no".to_string()], 0, false)
        .await
        .unwrap();

    let poll = db.get_poll(&poll_id).await.unwrap().unwrap();
    assert!(
        poll.2,
        "poll should be treated as expired when expires_at <= now"
    );
}

#[tokio::test]
async fn test_vote_in_poll_rejects_when_expires_at_has_passed() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-expired-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .create_poll(&status.id, &["yes".to_string(), "no".to_string()], 0, false)
        .await
        .unwrap();
    let options = db.get_poll_options(&poll_id).await.unwrap();
    let first_option_id = options[0].0.clone();

    let result = db
        .vote_in_poll(
            &poll_id,
            "alice@remote.example",
            std::slice::from_ref(&first_option_id),
        )
        .await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_replace_poll_for_status_preserves_poll_id_and_votes_for_matching_options() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-replace".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "alice@remote.example".to_string(),
        is_local: false,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Timeline,
        created_at: Utc::now(),
        fetched_at: Some(Utc::now()),
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .replace_poll_for_status(
            &status.id,
            "2099-01-10T00:00:00Z",
            false,
            false,
            1,
            1,
            &[("tea".to_string(), 1), ("coffee".to_string(), 0)],
        )
        .await
        .unwrap();
    let tea_option_id = db
        .get_poll_options(&poll_id)
        .await
        .unwrap()
        .into_iter()
        .find(|(_id, title, _votes_count)| title == "tea")
        .map(|(id, _title, _votes_count)| id)
        .expect("tea option");
    db.vote_in_poll(&poll_id, "bob@example.com", &[tea_option_id])
        .await
        .unwrap();

    let updated_poll_id = db
        .replace_poll_for_status(
            &status.id,
            "2099-01-11T00:00:00Z",
            false,
            false,
            2,
            2,
            &[("tea".to_string(), 2), ("juice".to_string(), 0)],
        )
        .await
        .unwrap();

    assert_eq!(updated_poll_id, poll_id);
    let own_votes = db
        .get_user_poll_votes(&poll_id, "bob@example.com")
        .await
        .unwrap();
    assert_eq!(own_votes.len(), 1);
    let options = db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0].1, "tea");
    assert_eq!(options[0].2, 2);
    assert_eq!(options[1].1, "juice");
}

#[tokio::test]
async fn test_record_remote_poll_vote_allows_additional_choices_for_multiple_poll() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/poll-remote-multi".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let poll_id = db
        .create_poll(&status.id, &["a".to_string(), "b".to_string()], 600, true)
        .await
        .unwrap();
    let options = db.get_poll_options(&poll_id).await.unwrap();
    let first_option_id = options[0].0.clone();
    let second_option_id = options[1].0.clone();

    assert!(
        db.record_remote_poll_vote(
            &poll_id,
            "alice@remote.example",
            std::slice::from_ref(&first_option_id),
        )
        .await
        .unwrap()
    );
    assert!(
        db.record_remote_poll_vote(
            &poll_id,
            "alice@remote.example",
            std::slice::from_ref(&second_option_id),
        )
        .await
        .unwrap()
    );
    assert!(
        !db.record_remote_poll_vote(
            &poll_id,
            "alice@remote.example",
            std::slice::from_ref(&second_option_id),
        )
        .await
        .unwrap()
    );

    let options_after = db.get_poll_options(&poll_id).await.unwrap();
    assert_eq!(options_after[0].2, 1);
    assert_eq!(options_after[1].2, 1);
    let poll_after = db.get_poll(&poll_id).await.unwrap().unwrap();
    assert_eq!(poll_after.4, 2);
    assert_eq!(poll_after.5, 1);
}

#[tokio::test]
async fn test_reserve_idempotency_key_reclaims_stale_pending_reservation() {
    let (db, _temp_dir) = create_test_db().await;
    let endpoint = "/api/v1/statuses";
    let key = "stale-pending-key";

    assert!(db.reserve_idempotency_key(endpoint, key).await.unwrap());
    assert!(!db.reserve_idempotency_key(endpoint, key).await.unwrap());

    db.backdate_pending_idempotency_key_for_test(endpoint, key, 10)
        .await
        .unwrap();

    assert!(db.reserve_idempotency_key(endpoint, key).await.unwrap());
}

#[tokio::test]
async fn test_attach_media_to_status_rejects_reassign_to_another_status() {
    let (db, _temp_dir) = create_test_db().await;

    let first_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/first".to_string(),
        content: "<p>First status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let second_status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/second".to_string(),
        content: "<p>Second status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&first_status).await.unwrap();
    db.insert_status(&second_status).await.unwrap();

    let media_id = EntityId::new_string();
    db.insert_media(&MediaAttachment {
        id: media_id.clone(),
        status_id: Some(first_status.id.clone()),
        s3_key: format!("media/{}.png", media_id),
        thumbnail_s3_key: None,
        content_type: "image/png".to_string(),
        file_size: 100,
        description: None,
        blurhash: None,
        width: Some(1),
        height: Some(1),
        focus_x: None,
        focus_y: None,
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    let result = db
        .attach_media_to_status(&media_id, &second_status.id)
        .await;
    assert!(result.is_err());

    let media = db.get_media(&media_id).await.unwrap().unwrap();
    assert_eq!(media.status_id, Some(first_status.id.clone()));
}

#[tokio::test]
async fn test_replace_status_media_detaches_and_attaches_expected_media() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new_string(),
        uri: "https://example.com/status/media-replace".to_string(),
        content: "<p>status</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&status).await.unwrap();

    let keep_id = EntityId::new_string();
    let detach_id = EntityId::new_string();
    let attach_id = EntityId::new_string();
    for (media_id, status_id) in [
        (&keep_id, Some(status.id.clone())),
        (&detach_id, Some(status.id.clone())),
        (&attach_id, None),
    ] {
        db.insert_media(&MediaAttachment {
            id: media_id.clone(),
            status_id,
            s3_key: format!("media/{}.png", media_id),
            thumbnail_s3_key: None,
            content_type: "image/png".to_string(),
            file_size: 100,
            description: None,
            blurhash: None,
            width: Some(1),
            height: Some(1),
            focus_x: None,
            focus_y: None,
            created_at: Utc::now(),
        })
        .await
        .unwrap();
    }

    db.replace_status_media(&status.id, &[keep_id.clone(), attach_id.clone()])
        .await
        .unwrap();

    assert_eq!(
        db.get_media(&keep_id).await.unwrap().unwrap().status_id,
        Some(status.id.clone())
    );
    assert_eq!(
        db.get_media(&attach_id).await.unwrap().unwrap().status_id,
        Some(status.id.clone())
    );
    assert_eq!(
        db.get_media(&detach_id).await.unwrap().unwrap().status_id,
        None
    );
}

#[tokio::test]
async fn test_update_status_with_edit_snapshot_and_media_rolls_back_on_missing_status() {
    let (db, _temp_dir) = create_test_db().await;

    let media_id = EntityId::new_string();
    db.insert_media(&MediaAttachment {
        id: media_id.clone(),
        status_id: None,
        s3_key: format!("media/{}.png", media_id),
        thumbnail_s3_key: None,
        content_type: "image/png".to_string(),
        file_size: 100,
        description: None,
        blurhash: None,
        width: Some(1),
        height: Some(1),
        focus_x: None,
        focus_y: None,
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    let previous = Status {
        id: "missing-status-id".to_string(),
        uri: "https://example.com/status/missing".to_string(),
        content: "<p>before</p>".to_string(),
        content_warning: None,
        visibility: crate::data::StatusVisibility::Public,
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        quote_of_uri: None,
        persisted_reason: PersistedReason::Own,
        created_at: Utc::now(),
        fetched_at: None,
    };
    let mut updated = previous.clone();
    updated.content = "<p>after</p>".to_string();

    let error = db
        .update_status_with_edit_snapshot_and_media(
            &previous,
            &updated,
            Some(std::slice::from_ref(&media_id)),
        )
        .await
        .expect_err("missing status should fail");
    assert!(matches!(
        error,
        crate::error::AppError::NotFound | crate::error::AppError::Database(_)
    ));

    let media = db.get_media(&media_id).await.unwrap().unwrap();
    assert_eq!(media.status_id, None);
}

#[tokio::test]
async fn test_accept_follow_request_moves_to_followers() {
    let (db, _temp_dir) = create_test_db().await;

    db.insert_follow_request_with_actor_uri(
        "alice@remote.example",
        "https://remote.example/inbox",
        "https://remote.example/follows/1",
        Some("https://remote.example/users/alice"),
    )
    .await
    .unwrap();

    let accepted = db
        .accept_follow_request("alice@remote.example")
        .await
        .unwrap();
    assert!(accepted);
    assert!(!db.has_follow_request("alice@remote.example").await.unwrap());

    let followers = db.get_all_follower_addresses().await.unwrap();
    assert_eq!(followers, vec!["alice@remote.example".to_string()]);

    let inboxes = db.get_follower_inboxes().await.unwrap();
    assert_eq!(inboxes, vec!["https://remote.example/inbox".to_string()]);

    let followers = db.get_all_followers().await.unwrap();
    assert_eq!(
        followers[0].actor_uri.as_deref(),
        Some("https://remote.example/users/alice")
    );
}

#[tokio::test]
async fn test_accept_follow_request_returns_false_when_missing() {
    let (db, _temp_dir) = create_test_db().await;

    let accepted = db
        .accept_follow_request("missing@remote.example")
        .await
        .unwrap();
    assert!(!accepted);
}

#[tokio::test]
async fn test_accept_follow_request_rolls_back_on_follower_insert_failure() {
    let (db, _temp_dir) = create_test_db().await;

    db.insert_follower(&Follower {
        id: EntityId::new_string(),
        follower_address: "alice@remote.example".to_string(),
        actor_uri: None,
        inbox_uri: "https://existing.example/inbox".to_string(),
        uri: "https://existing.example/follows/1".to_string(),
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    db.insert_follow_request(
        "alice@remote.example",
        "https://remote.example/inbox",
        "https://remote.example/follows/2",
    )
    .await
    .unwrap();

    let result = db.accept_follow_request("alice@remote.example").await;
    assert!(result.is_err());
    assert!(db.has_follow_request("alice@remote.example").await.unwrap());

    let follow_request = db.get_follow_request("alice@remote.example").await.unwrap();
    assert_eq!(
        follow_request,
        Some((
            "https://remote.example/inbox".to_string(),
            "https://remote.example/follows/2".to_string()
        ))
    );
}

#[tokio::test]
async fn test_delete_follow_request_with_default_port_matches_equivalent_requester_address() {
    let (db, _temp_dir) = create_test_db().await;

    db.insert_follow_request(
        "alice@remote.example",
        "https://remote.example/inbox",
        "https://remote.example/follows/1",
    )
    .await
    .unwrap();

    let deleted = db
        .delete_follow_request_with_default_port("alice@remote.example:443", Some(443))
        .await
        .unwrap();
    assert!(deleted);
    assert!(!db.has_follow_request("alice@remote.example").await.unwrap());
}

#[tokio::test]
async fn test_delete_follow_request_by_address_and_uri_respects_follow_activity_uri() {
    let (db, _temp_dir) = create_test_db().await;

    db.insert_follow_request(
        "alice@remote.example",
        "https://remote.example/inbox",
        "https://remote.example/follows/1",
    )
    .await
    .unwrap();
    let deleted = db
        .delete_follow_request_by_address_and_uri(
            "alice@remote.example:443",
            "https://remote.example/follows/2",
            Some(443),
        )
        .await
        .unwrap();
    assert!(!deleted);

    assert!(db.has_follow_request("alice@remote.example").await.unwrap());

    let deleted = db
        .delete_follow_request_by_address_and_uri(
            "alice@remote.example:443",
            "https://remote.example/follows/1",
            Some(443),
        )
        .await
        .unwrap();
    assert!(deleted);

    assert!(!db.has_follow_request("alice@remote.example").await.unwrap());
}
