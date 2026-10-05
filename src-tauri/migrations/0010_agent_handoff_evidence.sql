-- These are immutable observations, never a second execution state machine.
CREATE TABLE agent_handoff (
    operation_id TEXT PRIMARY KEY NOT NULL REFERENCES agent_operation(id) ON DELETE RESTRICT,
    workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE RESTRICT,
    mission_id TEXT NOT NULL REFERENCES agent_mission(id) ON DELETE RESTRICT,
    body_json TEXT NOT NULL CHECK (json_valid(body_json))
) STRICT;
CREATE INDEX agent_handoff_mission ON agent_handoff(mission_id);
CREATE TRIGGER agent_handoff_no_update BEFORE UPDATE ON agent_handoff
BEGIN SELECT RAISE(ABORT, 'agent handoff is immutable'); END;
CREATE TRIGGER agent_handoff_no_delete BEFORE DELETE ON agent_handoff
BEGIN SELECT RAISE(ABORT, 'agent handoff is immutable'); END;

CREATE TABLE agent_evidence (
    id TEXT PRIMARY KEY NOT NULL,
    workspace_id TEXT NOT NULL REFERENCES workspace(id) ON DELETE RESTRICT,
    mission_id TEXT NOT NULL REFERENCES agent_mission(id) ON DELETE RESTRICT,
    request_json TEXT NOT NULL CHECK (json_valid(request_json)),
    body_json TEXT NOT NULL CHECK (json_valid(body_json)),
    raw_receipt_json TEXT NOT NULL CHECK (json_valid(raw_receipt_json))
) STRICT;
CREATE INDEX agent_evidence_workspace ON agent_evidence(workspace_id);
CREATE TRIGGER agent_evidence_no_update BEFORE UPDATE ON agent_evidence
BEGIN SELECT RAISE(ABORT, 'agent evidence is immutable'); END;
CREATE TRIGGER agent_evidence_no_delete BEFORE DELETE ON agent_evidence
BEGIN SELECT RAISE(ABORT, 'agent evidence is immutable'); END;
