//! Streaming event bus service
//!
//! Provides a broadcast-backed pub/sub bus for Mastodon-compatible SSE streams.

use axum::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{HashMap, HashSet};
use tokio::sync::{RwLock, broadcast};

use crate::error::AppError;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StreamTarget {
    User { account_id: String },
    Public,
    PublicLocal,
    Hashtag { hashtag: String },
    HashtagLocal { hashtag: String },
    List { list_id: String },
    Direct { account_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum StreamEvent {
    Update {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    Delete {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    Notification {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    FiltersChanged {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    Announcement {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    AnnouncementReaction {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    AnnouncementDelete {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
    Conversation {
        payload: Value,
        targets: Vec<StreamTarget>,
    },
}

impl StreamEvent {
    pub fn event_name(&self) -> &'static str {
        match self {
            Self::Update { .. } => "update",
            Self::Delete { .. } => "delete",
            Self::Notification { .. } => "notification",
            Self::FiltersChanged { .. } => "filters_changed",
            Self::Announcement { .. } => "announcement",
            Self::AnnouncementReaction { .. } => "announcement_reaction",
            Self::AnnouncementDelete { .. } => "announcement_delete",
            Self::Conversation { .. } => "conversation",
        }
    }

    pub fn payload(&self) -> &Value {
        match self {
            Self::Update { payload, .. }
            | Self::Delete { payload, .. }
            | Self::Notification { payload, .. }
            | Self::FiltersChanged { payload, .. }
            | Self::Announcement { payload, .. }
            | Self::AnnouncementReaction { payload, .. }
            | Self::AnnouncementDelete { payload, .. }
            | Self::Conversation { payload, .. } => payload,
        }
    }

    pub fn targets(&self) -> &[StreamTarget] {
        match self {
            Self::Update { targets, .. }
            | Self::Delete { targets, .. }
            | Self::Notification { targets, .. }
            | Self::FiltersChanged { targets, .. }
            | Self::Announcement { targets, .. }
            | Self::AnnouncementReaction { targets, .. }
            | Self::AnnouncementDelete { targets, .. }
            | Self::Conversation { targets, .. } => targets,
        }
    }
}

#[derive(Debug)]
pub struct EventReceiver {
    inner: broadcast::Receiver<StreamEvent>,
}

impl EventReceiver {
    fn new(inner: broadcast::Receiver<StreamEvent>) -> Self {
        Self { inner }
    }

    pub fn into_inner(self) -> broadcast::Receiver<StreamEvent> {
        self.inner
    }

    pub async fn recv(&mut self) -> Result<StreamEvent, broadcast::error::RecvError> {
        self.inner.recv().await
    }
}

#[async_trait]
pub trait StreamingEventBus: Send + Sync {
    /// Subscribe to events for the authenticated user (notifications, home timeline updates).
    async fn subscribe_user(&self, account_id: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to the public (federated) timeline.
    async fn subscribe_public(&self) -> Result<EventReceiver, AppError>;
    /// Subscribe to the local-only public timeline.
    async fn subscribe_public_local(&self) -> Result<EventReceiver, AppError>;
    /// Subscribe to a hashtag timeline.
    async fn subscribe_hashtag(&self, hashtag: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to a local-only hashtag timeline.
    async fn subscribe_hashtag_local(&self, hashtag: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to a user-defined list timeline.
    async fn subscribe_list(&self, list_id: &str) -> Result<EventReceiver, AppError>;
    /// Subscribe to the direct-message (conversation) stream.
    async fn subscribe_direct(&self, account_id: &str) -> Result<EventReceiver, AppError>;
    /// Publish an event to all matching subscribers.
    async fn publish(&self, event: StreamEvent) -> Result<(), AppError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum StreamKey {
    User(String),
    Public,
    PublicLocal,
    Hashtag(String),
    HashtagLocal(String),
    List(String),
    Direct(String),
}

impl StreamKey {
    fn from_target(target: &StreamTarget) -> Self {
        match target {
            StreamTarget::User { account_id } => Self::User(account_id.clone()),
            StreamTarget::Public => Self::Public,
            StreamTarget::PublicLocal => Self::PublicLocal,
            StreamTarget::Hashtag { hashtag } => Self::Hashtag(hashtag.clone()),
            StreamTarget::HashtagLocal { hashtag } => Self::HashtagLocal(hashtag.clone()),
            StreamTarget::List { list_id } => Self::List(list_id.clone()),
            StreamTarget::Direct { account_id } => Self::Direct(account_id.clone()),
        }
    }
}

pub struct BroadcastEventBus {
    channel_capacity: usize,
    channels: RwLock<HashMap<StreamKey, broadcast::Sender<StreamEvent>>>,
}

impl BroadcastEventBus {
    pub fn new(channel_capacity: usize) -> Self {
        Self {
            channel_capacity: channel_capacity.max(1),
            channels: RwLock::new(HashMap::new()),
        }
    }

    async fn prune_empty_channels(&self) {
        let mut channels = self.channels.write().await;
        channels.retain(|_, sender| sender.receiver_count() > 0);
    }

    async fn get_or_create_sender(&self, key: StreamKey) -> broadcast::Sender<StreamEvent> {
        if let Some(sender) = self.channels.read().await.get(&key) {
            return sender.clone();
        }

        let mut channels = self.channels.write().await;
        channels
            .entry(key)
            .or_insert_with(|| {
                let (sender, _receiver) = broadcast::channel(self.channel_capacity);
                sender
            })
            .clone()
    }

    async fn get_sender(&self, key: &StreamKey) -> Option<broadcast::Sender<StreamEvent>> {
        self.channels.read().await.get(key).cloned()
    }

    async fn subscribe_key(&self, key: StreamKey) -> Result<EventReceiver, AppError> {
        // Reclaim channels whose subscribers disconnected before creating new keys.
        self.prune_empty_channels().await;
        let sender = self.get_or_create_sender(key).await;
        Ok(EventReceiver::new(sender.subscribe()))
    }
}

#[async_trait]
impl StreamingEventBus for BroadcastEventBus {
    async fn subscribe_user(&self, account_id: &str) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::User(account_id.trim().to_string()))
            .await
    }

    async fn subscribe_public(&self) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::Public).await
    }

    async fn subscribe_public_local(&self) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::PublicLocal).await
    }

    async fn subscribe_hashtag(&self, hashtag: &str) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::Hashtag(
            hashtag.trim_start_matches('#').to_ascii_lowercase(),
        ))
        .await
    }

    async fn subscribe_hashtag_local(&self, hashtag: &str) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::HashtagLocal(
            hashtag.trim_start_matches('#').to_ascii_lowercase(),
        ))
        .await
    }

    async fn subscribe_list(&self, list_id: &str) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::List(list_id.trim().to_string()))
            .await
    }

    async fn subscribe_direct(&self, account_id: &str) -> Result<EventReceiver, AppError> {
        self.subscribe_key(StreamKey::Direct(account_id.trim().to_string()))
            .await
    }

    async fn publish(&self, event: StreamEvent) -> Result<(), AppError> {
        let keys: HashSet<StreamKey> = event.targets().iter().map(StreamKey::from_target).collect();

        for key in keys {
            if let Some(sender) = self.get_sender(&key).await {
                let _ = sender.send(event.clone());
            }
        }
        self.prune_empty_channels().await;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{BroadcastEventBus, StreamEvent, StreamTarget, StreamingEventBus};
    use serde_json::json;
    use tokio::time::{Duration, timeout};

    #[tokio::test]
    async fn publishes_to_public_subscribers() {
        let bus = BroadcastEventBus::new(16);
        let mut receiver = bus.subscribe_public().await.expect("subscribe");

        let event = StreamEvent::Update {
            payload: json!({"id": "status-1"}),
            targets: vec![StreamTarget::Public],
        };
        bus.publish(event.clone()).await.expect("publish");

        let received = timeout(Duration::from_secs(1), receiver.recv())
            .await
            .expect("receive timeout")
            .expect("receiver open");

        assert!(matches!(received, StreamEvent::Update { .. }));
    }

    #[tokio::test]
    async fn routes_to_matching_user_only() {
        let bus = BroadcastEventBus::new(16);
        let mut alice = bus.subscribe_user("alice").await.expect("alice subscribe");
        let mut bob = bus.subscribe_user("bob").await.expect("bob subscribe");

        let event = StreamEvent::Notification {
            payload: json!({"id": "notification-1"}),
            targets: vec![StreamTarget::User {
                account_id: "alice".to_string(),
            }],
        };
        bus.publish(event).await.expect("publish");

        let alice_received = timeout(Duration::from_secs(1), alice.recv())
            .await
            .expect("alice receive timeout")
            .expect("alice receiver open");
        assert!(matches!(alice_received, StreamEvent::Notification { .. }));

        let bob_received = timeout(Duration::from_millis(100), bob.recv()).await;
        assert!(bob_received.is_err(), "bob should not receive alice event");
    }

    #[tokio::test]
    async fn prunes_stale_channels_when_subscribing_new_key() {
        let bus = BroadcastEventBus::new(16);

        let receiver = bus
            .subscribe_hashtag("old-tag")
            .await
            .expect("subscribe old-tag");
        drop(receiver);

        let _receiver = bus
            .subscribe_hashtag("new-tag")
            .await
            .expect("subscribe new-tag");

        let channel_count = bus.channels.read().await.len();
        assert_eq!(
            channel_count, 1,
            "stale channels should be reclaimed on subsequent subscriptions"
        );
    }
}
