-- Provider Profiles keep a stable non-secret Provider Template identity.
-- Existing OpenAI-compatible rows map to the generic template; Ollama rows
-- map to its native template without changing their endpoint or model.

ALTER TABLE provider_profile
ADD COLUMN provider_id TEXT NOT NULL DEFAULT 'openai-compatible';

UPDATE provider_profile
SET provider_id = 'ollama'
WHERE dialect = 'ollama_chat';

ALTER TABLE context_snapshot
ADD COLUMN provider_id TEXT;

ALTER TABLE context_snapshot
ADD COLUMN template_revision INTEGER CHECK (template_revision IS NULL OR template_revision > 0);

ALTER TABLE context_snapshot
ADD COLUMN stream_protocol TEXT;

ALTER TABLE context_snapshot
ADD COLUMN auth_placement TEXT;

ALTER TABLE context_snapshot
ADD COLUMN auth_header_name TEXT;

ALTER TABLE context_snapshot
ADD COLUMN additional_headers_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(additional_headers_json));

-- Historical snapshots predate Provider Templates, so their resolved template,
-- protocol, and authentication semantics are unknowable. Leave those columns
-- NULL instead of rewriting immutable evidence. New snapshots always persist
-- the complete resolved metadata group.
