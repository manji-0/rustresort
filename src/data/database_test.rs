//! Database tests

use super::*;
use chrono::Utc;
use sqlx::SqlitePool;
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
        id: EntityId::new().0,
        name: "Test App".to_string(),
        website: None,
        redirect_uri: "https://example.com/callback".to_string(),
        client_id: EntityId::new().0,
        client_secret: EntityId::new().0,
        scopes: "read write".to_string(),
        created_at: Utc::now(),
    }
}

fn test_oauth_token(app_id: &str, access_token: &str) -> OAuthToken {
    OAuthToken {
        id: EntityId::new().0,
        app_id: app_id.to_string(),
        access_token: access_token.to_string(),
        grant_type: "authorization_code".to_string(),
        scopes: "read write".to_string(),
        created_at: Utc::now(),
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
            id, app_id, access_token, grant_type, scopes, created_at, revoked
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&legacy_token.id)
    .bind(&legacy_token.app_id)
    .bind(&legacy_token.access_token)
    .bind(&legacy_token.grant_type)
    .bind(&legacy_token.scopes)
    .bind(&legacy_token.created_at)
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
            id, app_id, access_token, grant_type, scopes, created_at, revoked
        ) VALUES (?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&legacy_token.id)
    .bind(&legacy_token.app_id)
    .bind(&legacy_token.access_token)
    .bind(&legacy_token.grant_type)
    .bind(&legacy_token.scopes)
    .bind(&legacy_token.created_at)
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
async fn test_account_upsert_and_get() {
    let (db, _temp_dir) = create_test_db().await;

    let account = Account {
        id: EntityId::new().0,
        username: "testuser".to_string(),
        display_name: Some("Test User".to_string()),
        note: Some("Test bio".to_string()),
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
        id: EntityId::new().0,
        username: "first".to_string(),
        display_name: Some("First".to_string()),
        note: None,
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "first_private_key".to_string(),
        public_key_pem: "first_public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };

    let second = Account {
        id: EntityId::new().0,
        username: "second".to_string(),
        display_name: Some("Second".to_string()),
        note: None,
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
        id: EntityId::new().0,
        username: "patch-user".to_string(),
        display_name: Some("Patch User".to_string()),
        note: Some("original note".to_string()),
        avatar_s3_key: None,
        header_s3_key: None,
        private_key_pem: "private_key".to_string(),
        public_key_pem: "public_key".to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    db.upsert_account(&account).await.unwrap();

    let updated = db
        .patch_account_profile(&account.id, None, None, Utc::now())
        .await
        .unwrap();
    assert!(updated);

    let stored = db.get_account().await.unwrap().unwrap();
    assert_eq!(stored.display_name, Some("Patch User".to_string()));
    assert_eq!(stored.note, Some("original note".to_string()));
}

#[tokio::test]
async fn test_status_crud() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/123".to_string(),
        content: "<p>Hello, world!</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        target_address: "user@example.com".to_string(),
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
async fn test_insert_follow_if_absent_deduplicates_default_port_variants() {
    let (db, _temp_dir) = create_test_db().await;

    let first = Follow {
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
        uri: "https://example.com/follows/default-port-first".to_string(),
        created_at: Utc::now(),
    };
    let second = Follow {
        id: EntityId::new().0,
        target_address: "alice@remote.example".to_string(),
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
            id: EntityId::new().0,
            target_address: "alice@remote.example:443".to_string(),
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
            id: EntityId::new().0,
            target_address: "alice@remote.example".to_string(),
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
        id: EntityId::new().0,
        follower_address: "follower@example.com".to_string(),
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
        id: EntityId::new().0,
        follower_address: "bob@remote.example:443".to_string(),
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
        id: EntityId::new().0,
        follower_address: "bob@remote.example".to_string(),
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
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
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
        id: EntityId::new().0,
        target_address: "alice@remote.example".to_string(),
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
        id: EntityId::new().0,
        target_address: "alice@remote.example:80".to_string(),
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
        id: EntityId::new().0,
        target_address: "Alice@Remote.EXAMPLE".to_string(),
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
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
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
        id: EntityId::new().0,
        target_address: "alice@remote.example".to_string(),
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
async fn test_block_account_removes_follow_for_default_port_variant() {
    let (db, _temp_dir) = create_test_db().await;

    let follow = Follow {
        id: EntityId::new().0,
        target_address: "alice@remote.example:443".to_string(),
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
async fn test_notification_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let notification = Notification {
        id: EntityId::new().0,
        notification_type: "mention".to_string(),
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
    assert_eq!(notifications[0].notification_type, "mention");

    // Mark as read
    db.mark_notification_read(&notification.id).await.unwrap();
    let notifications = db.get_notifications(10, None, true).await.unwrap();
    assert_eq!(notifications.len(), 0);
}

#[tokio::test]
async fn test_favourite_operations() {
    let (db, _temp_dir) = create_test_db().await;

    // Create a status first (required for foreign key)
    let status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/fav".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/bookmark".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/repost".to_string(),
        content: "<p>Test</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let reply = Status {
        id: "pin-mute-reply".to_string(),
        uri: "https://example.com/status/pin-mute-reply".to_string(),
        content: "<p>Pin and mute reply</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(root.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
async fn test_status_reply_lookup_and_edit_history_operations() {
    let (db, _temp_dir) = create_test_db().await;

    let parent = Status {
        id: "parent-status".to_string(),
        uri: "https://example.com/status/parent".to_string(),
        content: "<p>Parent</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let child_a = Status {
        id: "child-a".to_string(),
        uri: "https://example.com/status/child-a".to_string(),
        content: "<p>Child A</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(parent.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let child_b = Status {
        id: "child-b".to_string(),
        uri: "https://example.com/status/child-b".to_string(),
        content: "<p>Child B</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: Some(parent.uri.clone()),
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
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
            visibility: "public".to_string(),
            language: Some("en".to_string()),
            account_address: "".to_string(),
            is_local: true,
            in_reply_to_uri: None,
            boost_of_uri: None,
            persisted_reason: "own".to_string(),
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
async fn test_insert_status_with_media_attaches_all_media_atomically() {
    let (db, _temp_dir) = create_test_db().await;

    let status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/with-media".to_string(),
        content: "<p>Status with media</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    let media_ids = vec![EntityId::new().0, EntityId::new().0];
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
        id: EntityId::new().0,
        uri: "https://example.com/status/missing-media".to_string(),
        content: "<p>Status with missing media</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };

    let result = db
        .insert_status_with_media(&status, &[EntityId::new().0])
        .await;
    assert!(result.is_err());
    assert!(db.get_status(&status.id).await.unwrap().is_none());
}

#[tokio::test]
async fn test_insert_status_with_media_rolls_back_when_media_already_attached() {
    let (db, _temp_dir) = create_test_db().await;

    let existing_status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/existing".to_string(),
        content: "<p>Existing status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&existing_status).await.unwrap();

    let media_id = EntityId::new().0;
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
        created_at: Utc::now(),
    })
    .await
    .unwrap();

    let new_status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/new".to_string(),
        content: "<p>New status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/with-poll".to_string(),
        content: "<p>Status with poll</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/with-poll-missing-media".to_string(),
        content: "<p>Status with poll and missing media</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let poll_options = vec!["a".to_string(), "b".to_string()];

    let result = db
        .insert_status_with_media_and_poll(
            &status,
            &[EntityId::new().0],
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
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-duplicate-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-1".to_string(),
        content: "<p>Poll 1</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let status_2 = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-2".to_string(),
        content: "<p>Poll 2</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-multiple-second-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-immediate-expire".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/poll-expired-vote".to_string(),
        content: "<p>Poll</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
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
        id: EntityId::new().0,
        uri: "https://example.com/status/first".to_string(),
        content: "<p>First status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    let second_status = Status {
        id: EntityId::new().0,
        uri: "https://example.com/status/second".to_string(),
        content: "<p>Second status</p>".to_string(),
        content_warning: None,
        visibility: "public".to_string(),
        language: Some("en".to_string()),
        account_address: "".to_string(),
        is_local: true,
        in_reply_to_uri: None,
        boost_of_uri: None,
        persisted_reason: "own".to_string(),
        created_at: Utc::now(),
        fetched_at: None,
    };
    db.insert_status(&first_status).await.unwrap();
    db.insert_status(&second_status).await.unwrap();

    let media_id = EntityId::new().0;
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
async fn test_accept_follow_request_moves_to_followers() {
    let (db, _temp_dir) = create_test_db().await;

    db.insert_follow_request(
        "alice@remote.example",
        "https://remote.example/inbox",
        "https://remote.example/follows/1",
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
        id: EntityId::new().0,
        follower_address: "alice@remote.example".to_string(),
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
