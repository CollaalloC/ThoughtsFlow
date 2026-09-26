-- Drive content hydration from this workspace's references, including immutable
-- Receipt items. The composite manifest foreign key already proves ownership.
-- This adds an access path only; no historical evidence is rewritten.
CREATE INDEX idx_manifest_item_workspace_content
ON context_manifest_item(workspace_id, content_block_id);
