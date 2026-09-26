-- Complete the two constraints absent from the allowlisted pre-commit v5.
-- Canonical v5 already has both objects. SQLx wraps this script and its history
-- row in one transaction; a timestamp collision must fail without rewriting
-- historical checkpoint timestamps, Receipt bytes, or hashes.

-- Historical v1-v4 Receipt items remain NULL/NULL. New typed items must carry
-- both halves of their stable identity, and the identifier cannot be blank.
CREATE TRIGGER IF NOT EXISTS context_manifest_item_typed_source_identity
BEFORE INSERT ON context_manifest_item
WHEN (NEW.source_ref_kind IS NULL) <> (NEW.source_ref_id IS NULL)
  OR (
      NEW.source_ref_kind IS NOT NULL
      AND (
          length(trim(NEW.source_ref_kind)) = 0
          OR length(trim(NEW.source_ref_id)) = 0
      )
  )
BEGIN
    SELECT RAISE(
        ABORT,
        'typed context source identity must be paired and nonempty'
    );
END;

-- Compiler v4 selects the latest applicable checkpoint by persisted time.
-- A workspace-local UNIQUE key makes that ordering total and auditable instead
-- of falling back to caller-generated checkpoint ids for millisecond ties.
CREATE UNIQUE INDEX IF NOT EXISTS ux_context_checkpoint_workspace_created_at
ON context_checkpoint(workspace_id, created_at);
