ALTER TABLE domain_blocks ADD COLUMN severity TEXT NOT NULL DEFAULT 'suspend';
ALTER TABLE domain_blocks ADD COLUMN reject_media INTEGER NOT NULL DEFAULT 1;
ALTER TABLE domain_blocks ADD COLUMN reject_reports INTEGER NOT NULL DEFAULT 1;
ALTER TABLE domain_blocks ADD COLUMN private_comment TEXT;
ALTER TABLE domain_blocks ADD COLUMN public_comment TEXT;
ALTER TABLE domain_blocks ADD COLUMN obfuscate INTEGER NOT NULL DEFAULT 0;
