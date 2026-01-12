# API Specification

## Overview

RustResort provides two types of APIs:

1. **Mastodon-compatible API** - Compatibility with existing Mastodon clients
2. **ActivityPub API** - Fediverse federation

## Base URL Structure

```
https://example.com/
├── api/v1/          # Mastodon-compatible API
├── api/v2/          # Mastodon v2 API
├── oauth/           # OAuth authentication
├── users/{username} # ActivityPub actor
├── statuses/{id}    # ActivityPub object
├── .well-known/     # Well-known endpoints
└── nodeinfo/        # NodeInfo
```

## Authentication

### OAuth 2.0 Flow

```
1. Register app:    POST /api/v1/apps
2. Authorization:   GET /oauth/authorize
3. Get token:       POST /oauth/token
4. API access:      Authorization: Bearer <token>
```

### Scopes

| Scope | Description |
|-------|-------------|
| `read` | Read access (all) |
| `read:accounts` | Read account information |
| `read:statuses` | Read statuses |
| `read:notifications` | Read notifications |
| `write` | Write access (all) |
| `write:statuses` | Create statuses |
| `write:media` | Upload media |
| `write:favourites` | Favourite operations |
| `follow` | Manage follow relationships |
| `push` | Manage Web Push |

## Mastodon-Compatible API

### Accounts

#### GET /api/v1/accounts/:id
Get account information.

**Response:**
```json
{
  "id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "username": "alice",
  "acct": "alice",
  "display_name": "Alice",
  "locked": false,
  "bot": false,
  "created_at": "2024-01-01T00:00:00.000Z",
  "note": "<p>Hello, world!</p>",
  "url": "https://example.com/@alice",
  "avatar": "https://example.com/media/avatars/alice.jpg",
  "header": "https://example.com/media/headers/alice.jpg",
  "followers_count": 42,
  "following_count": 23,
  "statuses_count": 100,
  "fields": [
    {
      "name": "Website",
      "value": "<a href=\"https://alice.example.com\">alice.example.com</a>",
      "verified_at": "2024-01-01T00:00:00.000Z"
    }
  ]
}
```

#### GET /api/v1/accounts/verify_credentials
Get authenticated user's information.

#### PATCH /api/v1/accounts/update_credentials
Update profile.

**Parameters:**
- `display_name` - Display name
- `note` - Biography
- `avatar` - Avatar image
- `header` - Header image
- `locked` - Require follow approval
- `fields_attributes` - Profile fields

#### GET /api/v1/accounts/:id/statuses
Get account's statuses.

#### GET /api/v1/accounts/:id/followers
Get followers list.

#### GET /api/v1/accounts/:id/following
Get following list.

#### POST /api/v1/accounts/:id/follow
Follow account.

#### POST /api/v1/accounts/:id/unfollow
Unfollow account.

#### POST /api/v1/accounts/:id/block
Block account.

#### POST /api/v1/accounts/:id/unblock
Unblock account.

#### POST /api/v1/accounts/:id/mute
Mute account.

#### POST /api/v1/accounts/:id/unmute
Unmute account.

#### GET /api/v1/accounts/relationships
Get relationships with multiple accounts.

### Statuses

#### POST /api/v1/statuses
Create new status.

**Parameters:**
```json
{
  "status": "Hello, Fediverse!",
  "media_ids": ["01H8Y3VXPQM5JNABCDEFGHIJK"],
  "poll": {
    "options": ["Option A", "Option B"],
    "expires_in": 86400,
    "multiple": false
  },
  "in_reply_to_id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "sensitive": false,
  "spoiler_text": "",
  "visibility": "public",
  "language": "en"
}
```

**Response:**
```json
{
  "id": "01H8Y3VXPQM5JNABCDEFGHIJK",
  "uri": "https://example.com/statuses/01H8Y3VXPQM5JNABCDEFGHIJK",
  "created_at": "2024-01-01T12:00:00.000Z",
  "account": { ... },
  "content": "<p>Hello, Fediverse!</p>",
  "visibility": "public",
  "sensitive": false,
  "media_attachments": [],
  "mentions": [],
  "tags": [],
  "reblogs_count": 0,
  "favourites_count": 0,
  "replies_count": 0
}
```

#### GET /api/v1/statuses/:id
Get status.

#### DELETE /api/v1/statuses/:id
Delete status.

#### PUT /api/v1/statuses/:id
Edit status.

#### GET /api/v1/statuses/:id/context
Get status context (reply tree).

#### POST /api/v1/statuses/:id/reblog
Reblog (boost) status.

#### POST /api/v1/statuses/:id/unreblog
Unreblog status.

#### POST /api/v1/statuses/:id/favourite
Favourite status.

#### POST /api/v1/statuses/:id/unfavourite
Unfavourite status.

#### POST /api/v1/statuses/:id/bookmark
Bookmark status.

#### POST /api/v1/statuses/:id/unbookmark
Unbookmark status.

#### POST /api/v1/statuses/:id/pin
Pin status to profile.

#### POST /api/v1/statuses/:id/unpin
Unpin status.

### Timelines

#### GET /api/v1/timelines/home
Home timeline.

**Parameters:**
- `max_id` - Get statuses older than this ID
- `since_id` - Get statuses newer than this ID
- `min_id` - Get statuses from this ID onwards (reverse)
- `limit` - Number of results (default 20, max 40)

#### GET /api/v1/timelines/public
Public (federated) timeline.

**Parameters:**
- `local` - Local only
- `remote` - Remote only
- `only_media` - Media attachments only

#### GET /api/v1/timelines/tag/:hashtag
Hashtag timeline.

### Notifications

#### GET /api/v1/notifications
Get notifications list.

**Parameters:**
- `types[]` - Notification types to include
- `exclude_types[]` - Notification types to exclude

**Notification Types:**
- `mention` - Mentioned in status
- `reblog` - Status reblogged
- `favourite` - Status favourited
- `follow` - New follower
- `follow_request` - Follow request (if locked)
- `poll` - Poll ended
- `status` - New status from followed account

#### GET /api/v1/notifications/:id
Get specific notification.

#### POST /api/v1/notifications/clear
Clear all notifications.

#### POST /api/v1/notifications/:id/dismiss
Dismiss specific notification.

### Media

#### POST /api/v1/media
Upload media.

**Parameters:**
- `file` - File (multipart/form-data)
- `description` - Alt text
- `focus` - Focus point (x,y)

#### POST /api/v2/media
Upload media asynchronously (returns 202 while processing).

#### GET /api/v1/media/:id
Get media information.

#### PUT /api/v1/media/:id
Update media information.

### Search

#### GET /api/v2/search
Search.

**Parameters:**
- `q` - Search query
- `type` - Search type (accounts, hashtags, statuses)
- `resolve` - Attempt WebFinger resolution
- `limit` - Number of results

### Instance

#### GET /api/v1/instance
Get instance information.

```json
{
  "uri": "example.com",
  "title": "Example Instance",
  "short_description": "A friendly instance",
  "description": "Full description here...",
  "email": "admin@example.com",
  "version": "0.1.0 (compatible; RustResort 0.1.0)",
  "stats": {
    "user_count": 1,
    "status_count": 1000,
    "domain_count": 50
  },
  "thumbnail": "https://example.com/thumbnail.png",
  "languages": ["en", "ja"],
  "registrations": false,
  "configuration": {
    "statuses": {
      "max_characters": 5000,
      "max_media_attachments": 6
    },
    "media_attachments": {
      "supported_mime_types": ["image/jpeg", "image/png", "image/gif", "video/mp4"],
      "image_size_limit": 10485760,
      "video_size_limit": 41943040
    },
    "polls": {
      "max_options": 4,
      "max_characters_per_option": 50,
      "min_expiration": 300,
      "max_expiration": 2629746
    }
  }
}
```

#### GET /api/v2/instance
Instance information (v2 format).

### App Registration

#### POST /api/v1/apps
Register client application.

**Parameters:**
```json
{
  "client_name": "My App",
  "redirect_uris": "https://myapp.example.com/callback",
  "scopes": "read write follow push",
  "website": "https://myapp.example.com"
}
```

### Lists

#### GET /api/v1/lists
Get user's lists.

#### GET /api/v1/lists/:id
Get specific list.

#### POST /api/v1/lists
Create list.

#### PUT /api/v1/lists/:id
Update list.

#### DELETE /api/v1/lists/:id
Delete list.

#### GET /api/v1/lists/:id/accounts
Get accounts in list.

#### POST /api/v1/lists/:id/accounts
Add accounts to list.

#### DELETE /api/v1/lists/:id/accounts
Remove accounts from list.

### Filters

#### GET /api/v2/filters
Get filters.

#### GET /api/v2/filters/:id
Get specific filter.

#### POST /api/v2/filters
Create filter.

#### PUT /api/v2/filters/:id
Update filter.

#### DELETE /api/v2/filters/:id
Delete filter.

### Polls

#### GET /api/v1/polls/:id
Get poll.

#### POST /api/v1/polls/:id/votes
Vote in poll.

### Scheduled Statuses

#### GET /api/v1/scheduled_statuses
Get scheduled statuses.

#### GET /api/v1/scheduled_statuses/:id
Get specific scheduled status.

#### PUT /api/v1/scheduled_statuses/:id
Update scheduled status.

#### DELETE /api/v1/scheduled_statuses/:id
Cancel scheduled status.

### Bookmarks

#### GET /api/v1/bookmarks
Get bookmarked statuses.

### Favourites

#### GET /api/v1/favourites
Get favourited statuses.

### Blocks

#### GET /api/v1/blocks
Get blocked accounts.

### Mutes

#### GET /api/v1/mutes
Get muted accounts.

### Follow Requests

#### GET /api/v1/follow_requests
Get follow requests.

#### POST /api/v1/follow_requests/:id/authorize
Authorize follow request.

#### POST /api/v1/follow_requests/:id/reject
Reject follow request.

## Well-Known Endpoints

### GET /.well-known/webfinger
WebFinger for user discovery.

**Parameters:**
- `resource` - Format: `acct:username@domain`

**Response:**
```json
{
  "subject": "acct:alice@example.com",
  "aliases": [
    "https://example.com/@alice",
    "https://example.com/users/alice"
  ],
  "links": [
    {
      "rel": "http://webfinger.net/rel/profile-page",
      "type": "text/html",
      "href": "https://example.com/@alice"
    },
    {
      "rel": "self",
      "type": "application/activity+json",
      "href": "https://example.com/users/alice"
    }
  ]
}
```

### GET /.well-known/nodeinfo
NodeInfo discovery.

**Response:**
```json
{
  "links": [
    {
      "rel": "http://nodeinfo.diaspora.software/ns/schema/2.0",
      "href": "https://example.com/nodeinfo/2.0"
    }
  ]
}
```

### GET /.well-known/host-meta
Host-meta (XML).

## NodeInfo

### GET /nodeinfo/2.0
NodeInfo 2.0 format.

```json
{
  "version": "2.0",
  "software": {
    "name": "rustresort",
    "version": "0.1.0"
  },
  "protocols": ["activitypub"],
  "usage": {
    "users": {
      "total": 1,
      "activeMonth": 1,
      "activeHalfyear": 1
    },
    "localPosts": 1000
  },
  "openRegistrations": false
}
```

## ActivityPub API

See [FEDERATION.md](FEDERATION.md) for detailed ActivityPub specifications.

### Actor

#### GET /users/{username}
Get actor object.

**Headers:** `Accept: application/activity+json`

### Inbox

#### POST /users/{username}/inbox
Send activity to actor's inbox.

**Required:** HTTP Signature

#### POST /inbox
Shared inbox.

### Outbox

#### GET /users/{username}/outbox
Get actor's outbox (OrderedCollection).

### Collections

#### GET /users/{username}/followers
Followers collection.

#### GET /users/{username}/following
Following collection.

#### GET /users/{username}/collections/featured
Featured (pinned) posts collection.

### Object

#### GET /statuses/{id}
Get Note object.

**Headers:** `Accept: application/activity+json`

## Error Responses

Unified error format across all APIs:

```json
{
  "error": "Record not found",
  "error_description": "The requested resource could not be found"
}
```

### HTTP Status Codes

| Code | Meaning |
|------|---------|
| 200 | Success |
| 201 | Created |
| 202 | Accepted (async processing) |
| 400 | Bad request |
| 401 | Authentication error |
| 403 | Permission error |
| 404 | Resource not found |
| 410 | Resource deleted |
| 422 | Validation error |
| 429 | Rate limited |
| 500 | Server error |
| 503 | Service unavailable |

## Rate Limiting

| Endpoint | Limit |
|----------|-------|
| General API | 300 req/5min |
| Auth endpoints | 30 req/5min |
| Media upload | 30 req/30min |
| Status creation | 30 req/30min |

**Response Headers:**
- `X-RateLimit-Limit` - Limit value
- `X-RateLimit-Remaining` - Remaining requests
- `X-RateLimit-Reset` - Reset time

## Pagination

Link header pagination:

```
Link: <https://example.com/api/v1/timelines/home?max_id=123>; rel="next",
      <https://example.com/api/v1/timelines/home?min_id=456>; rel="prev"
```

## Streaming API

WebSocket connection (future implementation):
- `wss://example.com/api/v1/streaming`

**Streams:**
- `user` - User notifications
- `public` - Public timeline
- `public:local` - Local timeline
- `hashtag` - Hashtag timeline
- `direct` - Direct messages

## Related Documentation

- [FEDERATION.md](FEDERATION.md) - Federation specifications
- [AUTHENTICATION.md](AUTHENTICATION.md) - Authentication details
- [DEVELOPMENT.md](DEVELOPMENT.md) - Development guide
- [TESTING.md](TESTING.md) - Testing specifications
