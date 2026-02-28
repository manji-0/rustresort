-- Add focus point metadata for media attachments.
ALTER TABLE media_attachments
    ADD COLUMN focus_x REAL;

ALTER TABLE media_attachments
    ADD COLUMN focus_y REAL;
