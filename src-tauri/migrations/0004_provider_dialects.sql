-- The v1 `dialect` CHECK cannot be widened with ALTER TABLE. Add a new
-- authoritative protocol dialect instead of dropping a parent table that is
-- already referenced by immutable Runs and Receipts. The legacy column stays
-- populated for backwards-compatible schema readers; all v4 reads use
-- `protocol_dialect`.

ALTER TABLE provider_profile
ADD COLUMN protocol_dialect TEXT NOT NULL DEFAULT 'openai_chat_completions'
CHECK (protocol_dialect IN (
    'openai_chat_completions',
    'ollama_chat',
    'anthropic_messages',
    'google_generative_ai'
));

UPDATE provider_profile
SET protocol_dialect = dialect;
