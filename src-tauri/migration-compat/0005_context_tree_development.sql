-- Persisted active Context, draft overrides, branch revisions, and auditable
-- maintenance/checkpoint facts. This migration is additive: v1-v4 receipts
-- remain byte-for-byte immutable.

-- SQLite accepts a UNIQUE index as a composite parent key. The extra
-- workspace component lets every new reference prove workspace ownership.
CREATE UNIQUE INDEX ux_branch_pointer_id_workspace
ON branch_pointer(id, workspace_id);

-- v1 stored one overloaded source identifier. Keep it for historical readers,
-- and add an independent typed source reference plus mandatory-policy evidence
-- for compiler v4 receipts.
ALTER TABLE context_manifest_item
ADD COLUMN source_ref_kind TEXT
CHECK (source_ref_kind IS NULL OR source_ref_kind IN (
    'workspace_system', 'turn_prompt', 'model_run', 'content_block',
    'current_prompt', 'checkpoint_summary', 'branch_summary'
));

ALTER TABLE context_manifest_item
ADD COLUMN source_ref_id TEXT;

ALTER TABLE context_manifest_item
ADD COLUMN mandatory INTEGER NOT NULL DEFAULT 0 CHECK (mandatory IN (0, 1));

ALTER TABLE context_manifest
ADD COLUMN checkpoint_provenance_json TEXT
CHECK (checkpoint_provenance_json IS NULL OR json_valid(checkpoint_provenance_json));

ALTER TABLE context_manifest
ADD COLUMN branch_summary_provenance_json TEXT NOT NULL DEFAULT '[]'
CHECK (
    json_valid(branch_summary_provenance_json)
    AND json_type(branch_summary_provenance_json) = 'array'
);

CREATE TABLE workspace_context_cursor (
    workspace_id       TEXT PRIMARY KEY NOT NULL,
    active_run_id      TEXT,
    branch_pointer_id  TEXT,
    version            INTEGER NOT NULL CHECK (version >= 1),
    updated_at         INTEGER NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (active_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (branch_pointer_id, workspace_id)
        REFERENCES branch_pointer(id, workspace_id) ON DELETE RESTRICT,
    CHECK (branch_pointer_id IS NULL OR active_run_id IS NOT NULL)
) STRICT;

CREATE TRIGGER workspace_context_cursor_revision_guard
BEFORE UPDATE ON workspace_context_cursor
WHEN NEW.workspace_id IS NOT OLD.workspace_id
  OR NEW.version <> OLD.version + 1
  OR NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'context cursor requires one monotonic revision');
END;

CREATE TABLE branch_revision (
    branch_pointer_id  TEXT NOT NULL,
    workspace_id       TEXT NOT NULL,
    revision           INTEGER NOT NULL CHECK (revision >= 0),
    name               TEXT NOT NULL CHECK (length(trim(name)) > 0),
    head_run_id        TEXT NOT NULL,
    change_kind        TEXT NOT NULL CHECK (change_kind IN (
                           'migration_baseline', 'created', 'advanced', 'renamed'
                       )),
    created_at         INTEGER NOT NULL,
    PRIMARY KEY (branch_pointer_id, revision),
    FOREIGN KEY (branch_pointer_id, workspace_id)
        REFERENCES branch_pointer(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (head_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT
) STRICT;

CREATE UNIQUE INDEX ux_branch_revision_workspace
ON branch_revision(branch_pointer_id, workspace_id, revision);

-- Existing branch rows are a current-state baseline. Do not invent the
-- revisions that happened before v5.
INSERT INTO branch_revision (
    branch_pointer_id, workspace_id, revision, name, head_run_id,
    change_kind, created_at
)
SELECT id, workspace_id, version, name, head_run_id,
       'migration_baseline', updated_at
FROM branch_pointer;

CREATE TRIGGER branch_pointer_v5_revision_guard
BEFORE UPDATE ON branch_pointer
WHEN NEW.id IS NOT OLD.id
  OR NEW.workspace_id IS NOT OLD.workspace_id
  OR NEW.created_at IS NOT OLD.created_at
  OR NEW.version <> OLD.version + 1
  OR NEW.updated_at < OLD.updated_at
BEGIN
    SELECT RAISE(ABORT, 'branch pointer requires one monotonic revision');
END;

CREATE TRIGGER branch_pointer_v5_insert_revision
AFTER INSERT ON branch_pointer
BEGIN
    INSERT INTO branch_revision (
        branch_pointer_id, workspace_id, revision, name, head_run_id,
        change_kind, created_at
    )
    VALUES (
        NEW.id, NEW.workspace_id, NEW.version, NEW.name, NEW.head_run_id,
        'created', NEW.updated_at
    );
END;

CREATE TRIGGER branch_pointer_v5_update_revision
AFTER UPDATE ON branch_pointer
BEGIN
    INSERT INTO branch_revision (
        branch_pointer_id, workspace_id, revision, name, head_run_id,
        change_kind, created_at
    )
    VALUES (
        NEW.id, NEW.workspace_id, NEW.version, NEW.name, NEW.head_run_id,
        CASE WHEN NEW.head_run_id IS NOT OLD.head_run_id
             THEN 'advanced' ELSE 'renamed' END,
        NEW.updated_at
    );
END;

CREATE TRIGGER branch_revision_no_update
BEFORE UPDATE ON branch_revision
BEGIN
    SELECT RAISE(ABORT, 'branch revisions are immutable');
END;

CREATE TRIGGER branch_revision_no_delete
BEFORE DELETE ON branch_revision
BEGIN
    SELECT RAISE(ABORT, 'branch revisions are immutable');
END;

CREATE TABLE context_draft (
    workspace_id       TEXT PRIMARY KEY NOT NULL,
    parent_run_id      TEXT,
    version            INTEGER NOT NULL CHECK (version >= 1),
    consumed_by_run_id TEXT,
    updated_at         INTEGER NOT NULL,
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (parent_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (consumed_by_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE context_override_item (
    workspace_id      TEXT NOT NULL,
    position          INTEGER NOT NULL CHECK (position >= 0),
    operation         TEXT NOT NULL CHECK (operation IN ('pin', 'exclude')),
    source_kind       TEXT NOT NULL CHECK (length(trim(source_kind)) > 0),
    source_id         TEXT,
    content_block_id  TEXT,
    content_hash      TEXT,
    created_at        INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, position),
    FOREIGN KEY (workspace_id) REFERENCES context_draft(workspace_id) ON DELETE CASCADE,
    FOREIGN KEY (content_block_id) REFERENCES content_block(id) ON DELETE RESTRICT,
    CHECK ((content_block_id IS NULL) = (content_hash IS NULL)),
    CHECK (operation <> 'pin'
        OR (content_block_id IS NOT NULL AND length(content_hash) > 0)),
    CHECK (operation <> 'exclude'
        OR source_kind NOT IN ('workspace_system', 'current_prompt'))
) STRICT;

CREATE TRIGGER context_override_item_content_identity
BEFORE INSERT ON context_override_item
WHEN NEW.content_block_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM content_block b
     WHERE b.id = NEW.content_block_id
       AND b.content_hash = NEW.content_hash
 )
BEGIN
    SELECT RAISE(ABORT, 'context override content identity does not match');
END;

CREATE TRIGGER context_override_item_update_content_identity
BEFORE UPDATE ON context_override_item
WHEN NEW.content_block_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM content_block b
     WHERE b.id = NEW.content_block_id
       AND b.content_hash = NEW.content_hash
 )
BEGIN
    SELECT RAISE(ABORT, 'context override content identity does not match');
END;

CREATE TABLE context_maintenance_run (
    id                     TEXT PRIMARY KEY NOT NULL,
    workspace_id           TEXT NOT NULL,
    kind                   TEXT NOT NULL CHECK (kind IN ('compaction', 'branch_summary')),
    anchor_run_id          TEXT NOT NULL,
    branch_pointer_id      TEXT,
    branch_revision        INTEGER CHECK (branch_revision IS NULL OR branch_revision >= 0),
    first_kept_run_id      TEXT,
    source_run_ids_json    TEXT NOT NULL CHECK (
                               json_valid(source_run_ids_json)
                               AND json_type(source_run_ids_json) = 'array'
                           ),
    source_hash            TEXT NOT NULL CHECK (length(source_hash) > 0),
    provider_snapshot_json TEXT CHECK (
                               provider_snapshot_json IS NULL
                               OR json_valid(provider_snapshot_json)
                           ),
    request_json           TEXT NOT NULL CHECK (json_valid(request_json)),
    status                 TEXT NOT NULL CHECK (status IN (
                               'queued', 'running', 'completed', 'failed', 'cancelled',
                               'conflicted'
                           )),
    summary_block_id       TEXT,
    error_json             TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    created_at             INTEGER NOT NULL,
    started_at             INTEGER,
    finished_at            INTEGER,
    UNIQUE (id, workspace_id),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (anchor_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (branch_pointer_id, workspace_id, branch_revision)
        REFERENCES branch_revision(branch_pointer_id, workspace_id, revision)
        ON DELETE RESTRICT,
    FOREIGN KEY (first_kept_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (summary_block_id) REFERENCES content_block(id) ON DELETE RESTRICT,
    CHECK (started_at IS NULL OR started_at >= created_at),
    CHECK (finished_at IS NULL OR finished_at >= created_at),
    CHECK ((branch_pointer_id IS NULL) = (branch_revision IS NULL)),
    CHECK (
        (status = 'queued'
            AND started_at IS NULL AND finished_at IS NULL
            AND summary_block_id IS NULL AND error_json IS NULL)
        OR
        (status = 'running'
            AND started_at IS NOT NULL AND finished_at IS NULL
            AND summary_block_id IS NULL AND error_json IS NULL)
        OR
        (status = 'completed'
            AND started_at IS NOT NULL AND finished_at IS NOT NULL
            AND summary_block_id IS NOT NULL AND error_json IS NULL)
        OR
        (status IN ('failed', 'cancelled', 'conflicted')
            AND finished_at IS NOT NULL AND summary_block_id IS NULL)
    )
) STRICT;

CREATE TRIGGER context_maintenance_identity_immutable
BEFORE UPDATE ON context_maintenance_run
WHEN NEW.id IS NOT OLD.id
  OR NEW.workspace_id IS NOT OLD.workspace_id
  OR NEW.kind IS NOT OLD.kind
  OR NEW.anchor_run_id IS NOT OLD.anchor_run_id
  OR NEW.branch_pointer_id IS NOT OLD.branch_pointer_id
  OR NEW.branch_revision IS NOT OLD.branch_revision
  OR NEW.first_kept_run_id IS NOT OLD.first_kept_run_id
  OR NEW.source_run_ids_json IS NOT OLD.source_run_ids_json
  OR NEW.source_hash IS NOT OLD.source_hash
  OR NEW.provider_snapshot_json IS NOT OLD.provider_snapshot_json
  OR NEW.request_json IS NOT OLD.request_json
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'context maintenance request is immutable');
END;

CREATE TRIGGER context_maintenance_terminal_immutable
BEFORE UPDATE ON context_maintenance_run
WHEN OLD.status IN ('completed', 'failed', 'cancelled', 'conflicted')
BEGIN
    SELECT RAISE(ABORT, 'terminal context maintenance runs are immutable');
END;

CREATE TRIGGER context_maintenance_valid_transition
BEFORE UPDATE OF status ON context_maintenance_run
WHEN NOT (
       (OLD.status = 'queued' AND NEW.status IN ('running', 'failed', 'cancelled', 'conflicted'))
    OR (OLD.status = 'running' AND NEW.status IN ('completed', 'failed', 'cancelled', 'conflicted'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid context maintenance status transition');
END;

CREATE TRIGGER context_maintenance_no_delete
BEFORE DELETE ON context_maintenance_run
BEGIN
    SELECT RAISE(ABORT, 'context maintenance runs are auditable');
END;

CREATE TABLE context_checkpoint (
    id                   TEXT PRIMARY KEY NOT NULL,
    workspace_id         TEXT NOT NULL,
    maintenance_run_id   TEXT NOT NULL UNIQUE,
    kind                 TEXT NOT NULL CHECK (kind IN ('compaction', 'branch_summary')),
    anchor_run_id        TEXT NOT NULL,
    branch_pointer_id    TEXT,
    branch_revision      INTEGER CHECK (branch_revision IS NULL OR branch_revision >= 0),
    first_kept_run_id    TEXT,
    summary_block_id     TEXT NOT NULL,
    source_run_ids_json  TEXT NOT NULL CHECK (
                             json_valid(source_run_ids_json)
                             AND json_type(source_run_ids_json) = 'array'
                         ),
    source_hash          TEXT NOT NULL CHECK (length(source_hash) > 0),
    created_at           INTEGER NOT NULL,
    UNIQUE (id, workspace_id),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (maintenance_run_id, workspace_id)
        REFERENCES context_maintenance_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (anchor_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (branch_pointer_id, workspace_id, branch_revision)
        REFERENCES branch_revision(branch_pointer_id, workspace_id, revision)
        ON DELETE RESTRICT,
    FOREIGN KEY (first_kept_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (summary_block_id) REFERENCES content_block(id) ON DELETE RESTRICT,
    CHECK ((branch_pointer_id IS NULL) = (branch_revision IS NULL))
) STRICT;

CREATE TRIGGER context_checkpoint_matches_maintenance
BEFORE INSERT ON context_checkpoint
WHEN NOT EXISTS (
    SELECT 1 FROM context_maintenance_run m
    WHERE m.id = NEW.maintenance_run_id
      AND m.workspace_id = NEW.workspace_id
      AND m.status = 'completed'
      AND m.kind = NEW.kind
      AND m.anchor_run_id = NEW.anchor_run_id
      AND m.branch_pointer_id IS NEW.branch_pointer_id
      AND m.branch_revision IS NEW.branch_revision
      AND m.first_kept_run_id IS NEW.first_kept_run_id
      AND m.summary_block_id = NEW.summary_block_id
      AND m.source_run_ids_json = NEW.source_run_ids_json
      AND m.source_hash = NEW.source_hash
)
BEGIN
    SELECT RAISE(ABORT, 'checkpoint does not match completed maintenance evidence');
END;

CREATE TRIGGER context_checkpoint_no_update
BEFORE UPDATE ON context_checkpoint
BEGIN
    SELECT RAISE(ABORT, 'context checkpoints are immutable');
END;

CREATE TRIGGER context_checkpoint_no_delete
BEFORE DELETE ON context_checkpoint
BEGIN
    SELECT RAISE(ABORT, 'context checkpoints are immutable');
END;

-- This is explicit visibility evidence, not a timestamp heuristic. A
-- checkpoint is recorded for its source branch when it completes. A branch
-- created later copies only the source branch's then-visible rows, so branches
-- created before a checkpoint never inherit it retroactively.
CREATE TABLE branch_checkpoint_inheritance (
    workspace_id       TEXT NOT NULL,
    branch_pointer_id  TEXT NOT NULL,
    checkpoint_id      TEXT NOT NULL,
    inherited_at       INTEGER NOT NULL,
    PRIMARY KEY (branch_pointer_id, checkpoint_id),
    FOREIGN KEY (branch_pointer_id, workspace_id)
        REFERENCES branch_pointer(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (checkpoint_id, workspace_id)
        REFERENCES context_checkpoint(id, workspace_id) ON DELETE RESTRICT
) STRICT;

CREATE TRIGGER branch_checkpoint_inheritance_no_update
BEFORE UPDATE ON branch_checkpoint_inheritance
BEGIN
    SELECT RAISE(ABORT, 'branch checkpoint inheritance is immutable');
END;

CREATE TRIGGER branch_checkpoint_inheritance_no_delete
BEFORE DELETE ON branch_checkpoint_inheritance
BEGIN
    SELECT RAISE(ABORT, 'branch checkpoint inheritance is immutable');
END;

CREATE INDEX idx_cursor_active_run
ON workspace_context_cursor(active_run_id);

CREATE INDEX idx_branch_revision_workspace
ON branch_revision(workspace_id, branch_pointer_id, revision DESC);

CREATE INDEX idx_context_draft_parent
ON context_draft(workspace_id, parent_run_id);

CREATE INDEX idx_context_maintenance_workspace
ON context_maintenance_run(workspace_id, created_at, id);

CREATE INDEX idx_context_checkpoint_anchor
ON context_checkpoint(workspace_id, anchor_run_id, created_at, id);

CREATE INDEX idx_branch_checkpoint_workspace
ON branch_checkpoint_inheritance(workspace_id, branch_pointer_id, inherited_at, checkpoint_id);

CREATE INDEX idx_context_maintenance_recovery
ON context_maintenance_run(status) WHERE status = 'running';
