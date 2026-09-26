-- Orca owns execution. These rows retain local ownership and mutation receipts.
CREATE TABLE agent_mission (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE RESTRICT,
    body_json TEXT NOT NULL CHECK (json_valid(body_json)),
    coordinator_pane_key TEXT,
    consumer_generation INTEGER
) STRICT;

CREATE TABLE agent_operation (
    id TEXT PRIMARY KEY NOT NULL,
    mission_id TEXT NOT NULL REFERENCES agent_mission(id) ON DELETE RESTRICT,
    kind TEXT NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('pending', 'succeeded', 'failed', 'unknown')),
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    receipt_json TEXT NOT NULL DEFAULT 'null' CHECK (json_valid(receipt_json)),
    error TEXT,
    created_at TEXT NOT NULL
) STRICT;
CREATE INDEX agent_mission_workspace ON agent_mission(workspace_id);
CREATE INDEX agent_operation_mission ON agent_operation(mission_id, created_at);
