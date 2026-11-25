-- Capability boundary: older core-7 binaries cannot decode Remade receipts.
-- No votes are stored here: their exactly-once terminal result uses career_matches.
INSERT INTO career_schema_version(version) VALUES (8);
