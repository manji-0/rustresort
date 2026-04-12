# RustResort Authentication

## Overview

RustResort is a single-user ActivityPub server.

Authentication is fully built in:

- one local username
- one bootstrap password used on first startup
- signed session tokens for API and browser access
- passkeys (WebAuthn) for ongoing logins after setup

OAuth app registration, authorization code flow, and external GitHub login are not used.

## Bootstrapping

On the first startup, RustResort initializes the local account from configuration:

```toml
[auth]
username = "admin"
password = "change-this-on-first-start"
session_secret = "replace-with-at-least-32-random-bytes"
session_max_age = 604800

[admin]
display_name = "Admin"
# email defaults to instance.contact_email
# note = "Instance administrator"
```

Behavior:

1. If the account table is empty, RustResort creates the single local account and generates ActivityPub keys.
2. If no local password hash exists in settings, RustResort hashes `auth.password` and stores it.
3. If no WebAuthn user id exists in settings, RustResort creates one for passkey ceremonies.

After the first successful initialization, the password hash remains in the database. `auth.password` is only required again when bootstrapping a fresh database or intentionally resetting credentials by clearing the stored hash.

Operational policy for this project:

- use the configured password to bootstrap the account and sign in the first time
- register one or more passkeys from the settings UI
- treat passkeys as the normal day-to-day login method after setup
- keep the password only as a bootstrap or recovery path

## Login Paths

### Password login

`POST /auth/login`

Request:

```json
{
  "username": "admin",
  "password": "change-this-on-first-start"
}
```

Response:

```json
{
  "access_token": "<signed-session-token>",
  "token_type": "Bearer",
  "username": "admin",
  "auth_method": "password"
}
```

The response also sets a `session` cookie for browser use.

### Session inspection

`GET /auth/session`

Requires a valid `Authorization: Bearer <token>` header or `session` cookie.

### Logout

`POST /logout`

Clears the browser session cookie.

## Passkeys

RustResort supports passkeys with WebAuthn.

Browser flow:

1. Open `/login`
2. Sign in with the configured bootstrap password
3. Use `Register current device passkey`
4. Future logins should use `Sign in with passkey`

API flow:

- `POST /auth/passkeys/register/start`
- `POST /auth/passkeys/register/finish`
- `POST /auth/passkeys/auth/start`
- `POST /auth/passkeys/auth/finish`
- `GET /auth/passkeys`
- `DELETE /auth/passkeys/:id`

Passkey registration requires an authenticated session. Passkey authentication is available without an existing session once at least one credential has been registered.

For single-user deployments, the intended steady state is passkey-first, effectively passkey-only for routine access.

## Mastodon API authentication

Protected Mastodon-compatible endpoints accept the same built-in signed session token:

```bash
curl http://localhost:3000/api/v1/accounts/verify_credentials \
  -H "Authorization: Bearer <signed-session-token>"
```

This is intentionally local-session based. OAuth scopes are not enforced.

## Disabled legacy endpoints

These endpoints are intentionally not exposed:

- `POST /api/v1/apps`
- `GET /oauth/authorize`
- `POST /oauth/token`
- `POST /oauth/revoke`
- `POST /api/v1/accounts`

That keeps the server aligned with the single-user built-in authentication model.
