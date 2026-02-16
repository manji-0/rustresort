//! Database tests

use super::*;
use chrono::Utc;
use tempfile::TempDir;

/// Helper to create a test database
async fn create_test_db() -> (Database, TempDir) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db = Database::connect(&db_path).await.unwrap();
    (db, temp_dir)
}

#[tokio::test]
async fn test_database_connection() {
    let (_db, _temp_dir) = create_test_db().await;
    // Connection successful if we get here without panicking
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
    db.delete_follow("user@example.com").await.unwrap();
    let addresses = db.get_all_follow_addresses().await.unwrap();
    assert_eq!(addresses.len(), 0);
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
    db.delete_follower("follower@example.com").await.unwrap();
    let addresses = db.get_all_follower_addresses().await.unwrap();
    assert_eq!(addresses.len(), 0);
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
