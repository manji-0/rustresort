# RustResort フェデレーション設計

## 概要

このドキュメントでは、RustResortにおけるActivityPubフェデレーションの実装詳細を説明します。
GoToSocialのフェデレーション実装を参考に、Fediverseとの相互運用性を確保します。

## ActivityPub概要

ActivityPubはW3C勧告の分散型ソーシャルネットワーキングプロトコルです。
RustResortは以下の仕様に準拠します：

- [ActivityPub](https://www.w3.org/TR/activitypub/)
- [ActivityStreams 2.0](https://www.w3.org/TR/activitystreams-core/)
- [ActivityStreams Vocabulary](https://www.w3.org/TR/activitystreams-vocabulary/)
- [HTTP Signatures (draft-cavage-http-signatures)](https://tools.ietf.org/html/draft-cavage-http-signatures-12)
- [WebFinger (RFC 7033)](https://tools.ietf.org/html/rfc7033)

## アーキテクチャ

```
                                    ┌─────────────────┐
                                    │  Remote Server  │
                                    └────────┬────────┘
                                             │
                        HTTPS + HTTP Signatures
                                             │
┌────────────────────────────────────────────┼────────────────────────────────────────────┐
│                                RustResort                                               │
│                                            │                                            │
│  ┌──────────────┐                 ┌────────┴────────┐                 ┌──────────────┐ │
│  │  Transport   │◄───────────────►│   Federator     │◄───────────────►│    Queue     │ │
│  │   Layer      │                 │                 │                 │   (Tokio)    │ │
│  └──────┬───────┘                 └────────┬────────┘                 └──────────────┘ │
│         │                                  │                                            │
│    ┌────┴────┐                      ┌──────┴──────┐                                    │
│    │  HTTP   │                      │             │                                    │
│    │ Client  │              ┌───────┴───────┐     │                                    │
│    │         │              │               │     │                                    │
│    └─────────┘       ┌──────┴──────┐ ┌──────┴──────┐                                   │
│                      │ Dereferencer│ │  Delivery   │                                   │
│                      │             │ │  Worker     │                                   │
│                      └─────────────┘ └─────────────┘                                   │
└─────────────────────────────────────────────────────────────────────────────────────────┘
```

## HTTP Signatures

### 署名の生成

全てのoutgoing ActivityPubリクエストにHTTP Signatureを付与します。

```rust
/// HTTP Signature生成
pub struct HttpSignature {
    key_id: String,      // 公開鍵URI
    algorithm: String,   // rsa-sha256
    headers: Vec<String>,
    signature: String,
}

impl HttpSignature {
    pub fn sign(
        private_key: &RsaPrivateKey,
        key_id: &str,
        method: &str,
        path: &str,
        headers: &HeaderMap,
        body: Option<&[u8]>,
    ) -> Result<Self, SignatureError> {
        let date = Utc::now().to_rfc2822();
        let digest = body.map(|b| {
            format!("SHA-256={}", base64::encode(sha256(b)))
        });
        
        let signed_headers = vec![
            "(request-target)",
            "host",
            "date",
            // digestはbodyがある場合のみ
        ];
        
        // 署名文字列の構築
        let signing_string = build_signing_string(
            method, path, &date, host, digest.as_deref()
        );
        
        // RSA-SHA256で署名
        let signature = sign_rsa_sha256(private_key, &signing_string)?;
        
        Ok(Self {
            key_id: key_id.to_string(),
            algorithm: "rsa-sha256".to_string(),
            headers: signed_headers.iter().map(|s| s.to_string()).collect(),
            signature: base64::encode(&signature),
        })
    }
    
    pub fn to_header(&self) -> String {
        format!(
            r#"keyId="{}",algorithm="{}",headers="{}",signature="{}""#,
            self.key_id,
            self.algorithm,
            self.headers.join(" "),
            self.signature
        )
    }
}
```

### 署名の検証

incoming リクエストの署名を検証します。

```rust
/// HTTP Signature検証
pub async fn verify_signature(
    request: &Request,
    fetch_public_key: impl Fn(&str) -> Future<Output = Result<RsaPublicKey>>,
) -> Result<(), SignatureError> {
    // 1. Signatureヘッダーをパース
    let sig_header = request.headers().get("Signature")
        .ok_or(SignatureError::MissingHeader)?;
    let signature = HttpSignature::parse(sig_header)?;
    
    // 2. 公開鍵を取得（キャッシュまたはフェッチ）
    let public_key = fetch_public_key(&signature.key_id).await?;
    
    // 3. 署名文字列を再構築
    let signing_string = rebuild_signing_string(request, &signature.headers)?;
    
    // 4. 署名を検証
    verify_rsa_sha256(&public_key, &signing_string, &signature.signature)?;
    
    // 5. Date/Digestヘッダーの検証
    validate_date(request)?;
    validate_digest(request)?;
    
    Ok(())
}
```

### 署名対象ヘッダー

| ヘッダー | 説明 | 必須 |
|---------|------|-----|
| `(request-target)` | HTTPメソッドとパス | Yes |
| `host` | ホスト名 | Yes |
| `date` | リクエスト日時 | Yes |
| `digest` | ボディのSHA-256ハッシュ | POST時 |
| `content-type` | コンテンツタイプ | POST時 |

### Date検証

リクエストの`Date`ヘッダーが±30秒以内であることを確認。
リプレイ攻撃対策として重要。

## Activity処理

### Inboxへの受信

```rust
/// Inbox受信処理
pub async fn process_inbox(
    state: &AppState,
    actor_id: &str,
    activity: Activity,
    signature_verified: bool,
) -> Result<(), FederationError> {
    if !signature_verified {
        return Err(FederationError::InvalidSignature);
    }
    
    // アクティビティのactorを検証
    let actor = state.dereferencer.fetch_actor(&activity.actor).await?;
    
    match activity.activity_type.as_str() {
        "Create" => handle_create(state, &actor, activity).await,
        "Update" => handle_update(state, &actor, activity).await,
        "Delete" => handle_delete(state, &actor, activity).await,
        "Follow" => handle_follow(state, &actor, activity).await,
        "Accept" => handle_accept(state, &actor, activity).await,
        "Reject" => handle_reject(state, &actor, activity).await,
        "Undo" => handle_undo(state, &actor, activity).await,
        "Announce" => handle_announce(state, &actor, activity).await,
        "Like" => handle_like(state, &actor, activity).await,
        "Block" => handle_block(state, &actor, activity).await,
        "Move" => handle_move(state, &actor, activity).await,
        _ => {
            tracing::warn!("Unhandled activity type: {}", activity.activity_type);
            Ok(())
        }
    }
}
```

### Create（投稿作成）

```rust
async fn handle_create(
    state: &AppState,
    actor: &Account,
    activity: Activity,
) -> Result<(), FederationError> {
    let object = activity.object.ok_or(FederationError::MissingObject)?;
    
    match object.object_type.as_str() {
        "Note" | "Article" | "Question" => {
            // 既存のStatusかチェック
            if state.db.get_status_by_uri(&object.id).await?.is_some() {
                return Ok(()); // 重複
            }
            
            // Noteをパースしてステータスを作成
            let status = parse_note_to_status(state, actor, &object).await?;
            
            // データベースに保存
            state.db.insert_status(&status).await?;
            
            // メンションへの通知
            for mention in &status.mentions {
                if mention.target_account.is_local() {
                    state.notification_service.create_mention_notification(
                        &mention.target_account_id,
                        &status.id,
                    ).await?;
                }
            }
            
            Ok(())
        }
        _ => {
            tracing::warn!("Unhandled object type in Create: {}", object.object_type);
            Ok(())
        }
    }
}
```

### Follow（フォロー）

```rust
async fn handle_follow(
    state: &AppState,
    actor: &Account,
    activity: Activity,
) -> Result<(), FederationError> {
    let target_uri = activity.object_uri()
        .ok_or(FederationError::MissingObject)?;
    
    // ターゲットがローカルアカウントか確認
    let target = state.db.get_account_by_uri(&target_uri).await?
        .ok_or(FederationError::AccountNotFound)?;
    
    if !target.is_local() {
        return Err(FederationError::NotLocalAccount);
    }
    
    // 既存のフォロー関係をチェック
    if state.db.is_following(&actor.id, &target.id).await? {
        // 既にフォロー済み、Acceptを送信
        send_accept_follow(state, &target, &actor, &activity.id).await?;
        return Ok(());
    }
    
    if target.locked {
        // 承認制の場合はFollowRequestを作成
        let follow_request = FollowRequest {
            id: EntityId::new(),
            account_id: actor.id.clone(),
            target_account_id: target.id.clone(),
            uri: activity.id.clone(),
            // ...
        };
        state.db.insert_follow_request(&follow_request).await?;
        
        // 通知を作成
        state.notification_service.create_follow_request_notification(
            &target.id,
            &actor.id,
        ).await?;
    } else {
        // 自動承認
        let follow = Follow {
            id: EntityId::new(),
            account_id: actor.id.clone(),
            target_account_id: target.id.clone(),
            uri: activity.id.clone(),
            // ...
        };
        state.db.insert_follow(&follow).await?;
        
        // Acceptを送信
        send_accept_follow(state, &target, &actor, &activity.id).await?;
        
        // 通知を作成
        state.notification_service.create_follow_notification(
            &target.id,
            &actor.id,
        ).await?;
    }
    
    Ok(())
}
```

### Undo（取り消し）

```rust
async fn handle_undo(
    state: &AppState,
    actor: &Account,
    activity: Activity,
) -> Result<(), FederationError> {
    let object = activity.object.ok_or(FederationError::MissingObject)?;
    
    // オブジェクトのactorとアクティビティのactorが一致することを確認
    if object.actor != Some(actor.uri.clone()) {
        return Err(FederationError::ActorMismatch);
    }
    
    match object.object_type.as_str() {
        "Follow" => {
            let target_uri = object.object_uri()
                .ok_or(FederationError::MissingObject)?;
            let target = state.db.get_account_by_uri(&target_uri).await?
                .ok_or(FederationError::AccountNotFound)?;
            
            // フォローを削除
            state.db.delete_follow(&actor.id, &target.id).await?;
            Ok(())
        }
        "Like" => {
            if let Some(status_uri) = object.object_uri() {
                let status = state.db.get_status_by_uri(&status_uri).await?
                    .ok_or(FederationError::StatusNotFound)?;
                state.db.delete_favourite(&actor.id, &status.id).await?;
            }
            Ok(())
        }
        "Announce" => {
            // ブースト解除
            if let Some(status_uri) = object.object_uri() {
                state.db.delete_boost_by_uri(&actor.id, &status_uri).await?;
            }
            Ok(())
        }
        "Block" => {
            if let Some(target_uri) = object.object_uri() {
                let target = state.db.get_account_by_uri(&target_uri).await?
                    .ok_or(FederationError::AccountNotFound)?;
                state.db.delete_block(&actor.id, &target.id).await?;
            }
            Ok(())
        }
        _ => {
            tracing::warn!("Unhandled Undo object type: {}", object.object_type);
            Ok(())
        }
    }
}
```

## Activity配信

### 配信ワーカー

```rust
/// Activity配信ワーカー
pub struct DeliveryWorker {
    http_client: Arc<HttpClient>,
    queue: Arc<Queue>,
}

impl DeliveryWorker {
    /// 配信キューを処理
    pub async fn run(&self) {
        loop {
            match self.queue.dequeue_delivery().await {
                Some(task) => {
                    if let Err(e) = self.deliver(task).await {
                        tracing::error!("Delivery failed: {}", e);
                        // リトライキューに追加
                    }
                }
                None => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
    }
    
    async fn deliver(&self, task: DeliveryTask) -> Result<(), DeliveryError> {
        let body = serde_json::to_vec(&task.activity)?;
        
        // HTTP Signature付きでPOST
        let response = self.http_client
            .post(&task.inbox_url)
            .header("Content-Type", "application/activity+json")
            .signed(&task.actor_private_key, &task.actor_key_id)
            .body(body)
            .send()
            .await?;
        
        match response.status() {
            StatusCode::OK | StatusCode::ACCEPTED | StatusCode::NO_CONTENT => Ok(()),
            StatusCode::GONE => {
                // アカウントが削除されている
                tracing::info!("Remote actor gone: {}", task.inbox_url);
                Ok(())
            }
            status => Err(DeliveryError::HttpError(status)),
        }
    }
}
```

### 配信先の計算

```rust
/// 配信先Inboxを計算
pub async fn calculate_inboxes(
    state: &AppState,
    status: &Status,
    activity: &Activity,
) -> Vec<String> {
    let mut inboxes = HashSet::new();
    
    match status.visibility {
        Visibility::Public | Visibility::Unlisted => {
            // フォロワー全員
            let followers = state.db.get_followers(&status.account_id).await.unwrap_or_default();
            for follower in followers {
                if follower.is_remote() {
                    if let Some(inbox) = follower.inbox_uri.or(follower.shared_inbox_uri) {
                        inboxes.insert(inbox);
                    }
                }
            }
        }
        Visibility::FollowersOnly => {
            // フォロワーのみ
            let followers = state.db.get_followers(&status.account_id).await.unwrap_or_default();
            for follower in followers {
                if follower.is_remote() {
                    if let Some(inbox) = follower.inbox_uri.or(follower.shared_inbox_uri) {
                        inboxes.insert(inbox);
                    }
                }
            }
        }
        Visibility::Direct => {
            // メンションされたアカウントのみ
        }
    }
    
    // メンションされたリモートアカウント
    for mention in &status.mentions {
        if let Some(account) = &mention.target_account {
            if account.is_remote() {
                if let Some(inbox) = &account.inbox_uri {
                    inboxes.insert(inbox.clone());
                }
            }
        }
    }
    
    // リプライ先
    if let Some(reply_account) = &status.in_reply_to_account {
        if reply_account.is_remote() {
            if let Some(inbox) = &reply_account.inbox_uri {
                inboxes.insert(inbox.clone());
            }
        }
    }
    
    inboxes.into_iter().collect()
}
```

### SharedInbox最適化

同じインスタンスへの複数配信を最適化：

```rust
/// SharedInboxによる配信最適化
pub fn optimize_inboxes(inboxes: Vec<String>, accounts: &[Account]) -> Vec<String> {
    let mut inbox_map: HashMap<String, HashSet<String>> = HashMap::new();
    
    for account in accounts {
        if let Some(ref shared_inbox) = account.shared_inbox_uri {
            // SharedInboxが使える場合はそちらを優先
            inbox_map.entry(shared_inbox.clone())
                .or_default()
                .insert(account.inbox_uri.clone().unwrap_or_default());
        } else if let Some(ref inbox) = account.inbox_uri {
            inbox_map.entry(inbox.clone())
                .or_default();
        }
    }
    
    // SharedInboxでカバーされるInboxを除外
    // ...
    
    inbox_map.keys().cloned().collect()
}
```

## Dereferencing（リモートリソース取得）

### アクター取得

```rust
/// リモートアクターを取得（キャッシュ付き）
pub async fn fetch_actor(
    &self,
    uri: &str,
) -> Result<Account, DereferenceError> {
    // 1. キャッシュを確認
    if let Some(account) = self.cache.get_account(uri).await {
        // 有効期限内ならキャッシュを返す
        if !account.needs_refresh() {
            return Ok(account);
        }
    }
    
    // 2. データベースを確認
    if let Some(account) = self.db.get_account_by_uri(uri).await? {
        if !account.needs_refresh() {
            self.cache.set_account(uri, &account).await;
            return Ok(account);
        }
    }
    
    // 3. リモートからフェッチ
    let actor_json = self.http_client
        .get(uri)
        .header("Accept", "application/activity+json")
        .signed_get(&self.instance_actor_key)
        .send()
        .await?
        .json::<Value>()
        .await?;
    
    // 4. パースして保存
    let account = parse_actor_to_account(&actor_json)?;
    self.db.upsert_account(&account).await?;
    self.cache.set_account(uri, &account).await;
    
    Ok(account)
}
```

### WebFinger

```rust
/// WebFingerでアカウントを発見
pub async fn webfinger(
    &self,
    username: &str,
    domain: &str,
) -> Result<String, WebFingerError> {
    let url = format!(
        "https://{}/.well-known/webfinger?resource=acct:{}@{}",
        domain, username, domain
    );
    
    let response = self.http_client
        .get(&url)
        .header("Accept", "application/jrd+json")
        .send()
        .await?;
    
    let jrd: WebFingerResponse = response.json().await?;
    
    // self rel="self" type="application/activity+json"を探す
    for link in jrd.links {
        if link.rel == "self" && link.link_type == Some("application/activity+json".to_string()) {
            return Ok(link.href.unwrap_or_default());
        }
    }
    
    Err(WebFingerError::NoActivityPubLink)
}
```

## ドメインブロック

```rust
/// ドメインブロックチェック
pub fn is_domain_blocked(domain: &str, blocked_domains: &HashSet<String>) -> bool {
    // 完全一致
    if blocked_domains.contains(domain) {
        return true;
    }
    
    // サブドメインチェック
    let parts: Vec<&str> = domain.split('.').collect();
    for i in 1..parts.len() {
        let parent_domain = parts[i..].join(".");
        if blocked_domains.contains(&parent_domain) {
            return true;
        }
    }
    
    false
}
```

## 実装における考慮事項

### セキュリティ

1. **HTTP Signature必須**: 全てのincoming リクエストに署名を要求
2. **Date検証**: リプレイ攻撃対策
3. **アクター検証**: アクティビティのactorと署名鍵の所有者が一致することを確認
4. **ドメインブロック**: 悪意あるインスタンスからの通信をブロック

### パフォーマンス

1. **SharedInbox活用**: 配信数の削減
2. **公開鍵キャッシュ**: 署名検証の高速化
3. **バッチ配信**: 複数アクティビティの一括配信
4. **リトライ戦略**: 指数バックオフによるリトライ

### 互換性

GoToSocialの互換性考慮事項を参考に：

1. **Mastodon互換性**: 最も多いユーザーベースへの対応
2. **Pleroma/Akkoma互換性**: MFM等の拡張対応
3. **Misskey互換性**: 一部独自拡張への対応
4. **厳密なActivityPub準拠**: 標準仕様への準拠を優先

## テスト戦略

1. **ユニットテスト**: HTTP Signature生成/検証
2. **統合テスト**: 各Activity処理のエンドツーエンドテスト
3. **インターオペラビリティテスト**: Mastodon等との実際の通信テスト
4. **フェデレーションテスト**: 複数インスタンス間のテスト環境

## 次のステップ

- [DEVELOPMENT.md](./DEVELOPMENT.md) - 開発環境セットアップ
