-- ThoughsFlow schema v1. All persisted product facts use STRICT tables.
-- Foreign-key enforcement is also enabled on every repository connection.

CREATE TABLE workspace (
    id            TEXT PRIMARY KEY NOT NULL,
    title         TEXT NOT NULL CHECK (length(trim(title)) > 0),
    system_prompt TEXT NOT NULL DEFAULT '',
    created_at    INTEGER NOT NULL,
    updated_at    INTEGER NOT NULL,
    archived_at   INTEGER,
    CHECK (updated_at >= created_at),
    CHECK (archived_at IS NULL OR archived_at >= created_at)
) STRICT;

CREATE TABLE provider_profile (
    id              TEXT PRIMARY KEY NOT NULL,
    name            TEXT NOT NULL CHECK (length(trim(name)) > 0),
    dialect         TEXT NOT NULL CHECK (dialect IN ('openai_chat_completions', 'ollama_chat')),
    base_url        TEXT NOT NULL CHECK (length(trim(base_url)) > 0),
    default_model   TEXT NOT NULL CHECK (length(trim(default_model)) > 0),
    parameters_json TEXT NOT NULL DEFAULT '{}' CHECK (json_valid(parameters_json)),
    created_at      INTEGER NOT NULL,
    updated_at      INTEGER NOT NULL,
    CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE content_block (
    id           TEXT PRIMARY KEY NOT NULL,
    role         TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool', 'manual')),
    content      TEXT NOT NULL,
    content_hash TEXT NOT NULL CHECK (length(content_hash) > 0),
    created_at   INTEGER NOT NULL,
    UNIQUE (role, content_hash)
) STRICT;

-- parent_run_id is the only persisted topology edge. The composite foreign key
-- ensures that a branch cannot inherit a Run from another workspace.
CREATE TABLE turn (
    id              TEXT PRIMARY KEY NOT NULL,
    workspace_id    TEXT NOT NULL,
    parent_run_id   TEXT,
    prompt_block_id TEXT NOT NULL,
    title           TEXT NOT NULL DEFAULT '',
    created_at      INTEGER NOT NULL,
    deleted_at      INTEGER,
    UNIQUE (id, workspace_id),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (prompt_block_id) REFERENCES content_block(id) ON DELETE RESTRICT,
    FOREIGN KEY (parent_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    CHECK (deleted_at IS NULL OR deleted_at >= created_at)
) STRICT;

CREATE TABLE model_run (
    id                     TEXT PRIMARY KEY NOT NULL,
    turn_id                TEXT NOT NULL,
    workspace_id           TEXT NOT NULL,
    provider_profile_id    TEXT,
    model                  TEXT NOT NULL CHECK (length(trim(model)) > 0),
    status                 TEXT NOT NULL CHECK (status IN (
                               'queued', 'connecting', 'streaming', 'completed',
                               'cancelled', 'failed', 'interrupted'
                           )),
    output_markdown        TEXT NOT NULL DEFAULT '',
    reasoning_markdown     TEXT NOT NULL DEFAULT '',
    provider_snapshot_json TEXT NOT NULL CHECK (json_valid(provider_snapshot_json)),
    usage_json             TEXT CHECK (usage_json IS NULL OR json_valid(usage_json)),
    error_json             TEXT CHECK (error_json IS NULL OR json_valid(error_json)),
    created_at             INTEGER NOT NULL,
    started_at             INTEGER,
    finished_at            INTEGER,
    checkpointed_at        INTEGER,
    UNIQUE (id, workspace_id),
    FOREIGN KEY (turn_id, workspace_id)
        REFERENCES turn(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (provider_profile_id) REFERENCES provider_profile(id) ON DELETE RESTRICT,
    CHECK (started_at IS NULL OR started_at >= created_at),
    CHECK (finished_at IS NULL OR finished_at >= created_at),
    CHECK (checkpointed_at IS NULL OR checkpointed_at >= created_at),
    CHECK ((status IN ('completed', 'cancelled', 'failed', 'interrupted') AND finished_at IS NOT NULL)
        OR (status IN ('queued', 'connecting', 'streaming') AND finished_at IS NULL))
) STRICT;

CREATE TABLE context_manifest (
    id                TEXT PRIMARY KEY NOT NULL,
    workspace_id      TEXT NOT NULL,
    compiler_version  TEXT NOT NULL,
    strategy          TEXT NOT NULL,
    estimated_chars   INTEGER NOT NULL CHECK (estimated_chars >= 0),
    canonical_hash    TEXT NOT NULL CHECK (length(canonical_hash) > 0),
    warnings_json     TEXT NOT NULL DEFAULT '[]' CHECK (json_valid(warnings_json)),
    created_at        INTEGER NOT NULL,
    UNIQUE (id, workspace_id),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE context_manifest_item (
    manifest_id       TEXT NOT NULL,
    workspace_id      TEXT NOT NULL,
    position          INTEGER NOT NULL CHECK (position >= 0),
    source_id         TEXT,
    source_kind       TEXT NOT NULL CHECK (length(source_kind) > 0),
    role              TEXT NOT NULL CHECK (role IN ('system', 'user', 'assistant', 'tool')),
    content_block_id  TEXT NOT NULL,
    inclusion_reason  TEXT NOT NULL,
    PRIMARY KEY (manifest_id, position),
    FOREIGN KEY (manifest_id, workspace_id)
        REFERENCES context_manifest(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (content_block_id) REFERENCES content_block(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE context_snapshot (
    id                    TEXT PRIMARY KEY NOT NULL,
    run_id                TEXT NOT NULL UNIQUE,
    manifest_id           TEXT NOT NULL UNIQUE,
    workspace_id          TEXT NOT NULL,
    provider_profile_id   TEXT,
    provider              TEXT NOT NULL,
    model                 TEXT NOT NULL,
    base_url              TEXT NOT NULL,
    parameters_json       TEXT NOT NULL CHECK (json_valid(parameters_json)),
    request_json          TEXT NOT NULL CHECK (json_valid(request_json)),
    canonical_hash        TEXT NOT NULL CHECK (length(canonical_hash) > 0),
    created_at            INTEGER NOT NULL,
    FOREIGN KEY (run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (manifest_id, workspace_id)
        REFERENCES context_manifest(id, workspace_id) ON DELETE RESTRICT,
    FOREIGN KEY (provider_profile_id) REFERENCES provider_profile(id) ON DELETE RESTRICT
) STRICT;

CREATE TABLE branch_pointer (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL,
    name         TEXT NOT NULL CHECK (length(trim(name)) > 0),
    head_run_id  TEXT NOT NULL,
    version      INTEGER NOT NULL DEFAULT 0 CHECK (version >= 0),
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE (workspace_id, name),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (head_run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE decision_mark (
    id           TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL,
    run_id       TEXT NOT NULL,
    status       TEXT NOT NULL CHECK (status IN ('adopted', 'rejected', 'needs_validation')),
    reason       TEXT NOT NULL DEFAULT '',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL,
    UNIQUE (workspace_id, run_id),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT,
    FOREIGN KEY (run_id, workspace_id)
        REFERENCES model_run(id, workspace_id) ON DELETE RESTRICT,
    CHECK (updated_at >= created_at)
) STRICT;

CREATE TABLE view_state (
    workspace_id TEXT NOT NULL,
    view_key     TEXT NOT NULL,
    state_json   TEXT NOT NULL CHECK (json_valid(state_json)),
    updated_at   INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, view_key),
    FOREIGN KEY (workspace_id) REFERENCES workspace(id) ON DELETE RESTRICT
) STRICT;

CREATE INDEX idx_turn_workspace_created ON turn(workspace_id, created_at, id);
CREATE INDEX idx_turn_parent_run ON turn(parent_run_id);
CREATE INDEX idx_model_run_turn_created ON model_run(turn_id, created_at, id);
CREATE INDEX idx_model_run_recovery ON model_run(status) WHERE status IN ('connecting', 'streaming');
CREATE INDEX idx_manifest_item_content ON context_manifest_item(content_block_id);
CREATE INDEX idx_branch_pointer_workspace ON branch_pointer(workspace_id, updated_at DESC);
CREATE INDEX idx_decision_mark_workspace ON decision_mark(workspace_id, updated_at DESC);

-- Immutable content and receipts are evidence. They are never rewritten or
-- removed through the live repository.
CREATE TRIGGER content_block_no_update
BEFORE UPDATE ON content_block
BEGIN
    SELECT RAISE(ABORT, 'content blocks are immutable');
END;

CREATE TRIGGER content_block_no_delete
BEFORE DELETE ON content_block
BEGIN
    SELECT RAISE(ABORT, 'content blocks are immutable');
END;

CREATE TRIGGER context_manifest_no_update
BEFORE UPDATE ON context_manifest
BEGIN
    SELECT RAISE(ABORT, 'context manifests are immutable');
END;

CREATE TRIGGER context_manifest_no_delete
BEFORE DELETE ON context_manifest
BEGIN
    SELECT RAISE(ABORT, 'context manifests are immutable');
END;

CREATE TRIGGER context_manifest_item_no_update
BEFORE UPDATE ON context_manifest_item
BEGIN
    SELECT RAISE(ABORT, 'context manifest items are immutable');
END;

CREATE TRIGGER context_manifest_item_no_delete
BEFORE DELETE ON context_manifest_item
BEGIN
    SELECT RAISE(ABORT, 'context manifest items are immutable');
END;

CREATE TRIGGER context_snapshot_no_update
BEFORE UPDATE ON context_snapshot
BEGIN
    SELECT RAISE(ABORT, 'context snapshots are immutable');
END;

CREATE TRIGGER context_snapshot_no_delete
BEFORE DELETE ON context_snapshot
BEGIN
    SELECT RAISE(ABORT, 'context snapshots are immutable');
END;

-- Prompts and lineage are immutable after insertion. Title and tombstone are
-- the only editable Turn fields.
CREATE TRIGGER turn_immutable_history
BEFORE UPDATE ON turn
WHEN NEW.id IS NOT OLD.id
  OR NEW.workspace_id IS NOT OLD.workspace_id
  OR NEW.parent_run_id IS NOT OLD.parent_run_id
  OR NEW.prompt_block_id IS NOT OLD.prompt_block_id
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'turn prompt and lineage are immutable');
END;

CREATE TRIGGER turn_parent_run_must_be_branchable
BEFORE INSERT ON turn
WHEN NEW.parent_run_id IS NOT NULL
 AND NOT EXISTS (
     SELECT 1 FROM model_run r
     WHERE r.id = NEW.parent_run_id
       AND r.workspace_id = NEW.workspace_id
       AND (
           r.status = 'completed'
           OR (r.status IN ('cancelled', 'failed', 'interrupted') AND length(r.output_markdown) > 0)
       )
 )
BEGIN
    SELECT RAISE(ABORT, 'parent model run is not branchable');
END;

CREATE TRIGGER model_run_immutable_identity
BEFORE UPDATE ON model_run
WHEN NEW.id IS NOT OLD.id
  OR NEW.turn_id IS NOT OLD.turn_id
  OR NEW.workspace_id IS NOT OLD.workspace_id
  OR NEW.provider_profile_id IS NOT OLD.provider_profile_id
  OR NEW.model IS NOT OLD.model
  OR NEW.provider_snapshot_json IS NOT OLD.provider_snapshot_json
  OR NEW.created_at IS NOT OLD.created_at
BEGIN
    SELECT RAISE(ABORT, 'model run request identity is immutable');
END;

CREATE TRIGGER model_run_terminal_is_immutable
BEFORE UPDATE ON model_run
WHEN OLD.status IN ('completed', 'cancelled', 'failed', 'interrupted')
BEGIN
    SELECT RAISE(ABORT, 'terminal model runs are immutable');
END;

CREATE TRIGGER model_run_valid_transition
BEFORE UPDATE OF status ON model_run
WHEN NOT (
       (OLD.status = 'queued' AND NEW.status IN ('connecting', 'cancelled', 'failed', 'interrupted'))
    OR (OLD.status = 'connecting' AND NEW.status IN ('streaming', 'cancelled', 'failed', 'interrupted'))
    OR (OLD.status = 'streaming' AND NEW.status IN ('streaming', 'completed', 'cancelled', 'failed', 'interrupted'))
)
BEGIN
    SELECT RAISE(ABORT, 'invalid model run status transition');
END;
