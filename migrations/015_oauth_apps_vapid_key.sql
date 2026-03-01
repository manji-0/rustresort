-- Add optional VAPID key column for OAuth applications

ALTER TABLE oauth_apps
ADD COLUMN vapid_key TEXT;
