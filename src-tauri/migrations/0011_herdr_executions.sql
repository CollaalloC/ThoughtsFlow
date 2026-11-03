-- Old Orca missions and immutable receipts are not rewritten or rebound.
-- Each new terminal binding belongs to one durable local launch operation.
CREATE TABLE agent_execution (
    operation_id TEXT PRIMARY KEY REFERENCES agent_operation(id),
    mission_id TEXT NOT NULL REFERENCES agent_mission(id),
    body_json TEXT NOT NULL CHECK (json_valid(body_json))
) STRICT;
CREATE INDEX agent_execution_mission ON agent_execution(mission_id);
CREATE TRIGGER agent_execution_no_update BEFORE UPDATE ON agent_execution
BEGIN SELECT RAISE(ABORT, 'agent execution binding is immutable'); END;
CREATE TRIGGER agent_execution_no_delete BEFORE DELETE ON agent_execution
BEGIN SELECT RAISE(ABORT, 'agent execution binding is immutable'); END;
