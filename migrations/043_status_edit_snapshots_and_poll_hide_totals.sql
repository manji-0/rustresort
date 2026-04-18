ALTER TABLE status_edits
    ADD COLUMN media_attachments_json TEXT;

ALTER TABLE status_edits
    ADD COLUMN poll_json TEXT;

ALTER TABLE status_edits
    ADD COLUMN quote_json TEXT;

ALTER TABLE polls
    ADD COLUMN hide_totals INTEGER NOT NULL DEFAULT 0;
