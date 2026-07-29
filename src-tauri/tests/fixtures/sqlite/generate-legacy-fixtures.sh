#!/usr/bin/env bash
set -euo pipefail

fixture_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
tauri_dir="$(cd "${fixture_dir}/../../.." && pwd)"
migration_dir="${tauri_dir}/migrations"

v1="${fixture_dir}/legacy-v1.sqlite"
v2="${fixture_dir}/legacy-v2.sqlite"
v3="${fixture_dir}/legacy-v3.sqlite"
v4="${fixture_dir}/legacy-v4.sqlite"

for fixture in "${v1}" "${v2}" "${v3}" "${v4}"; do
    if [[ -e "${fixture}" ]]; then
        echo "Refusing to overwrite frozen fixture: ${fixture}" >&2
        exit 1
    fi
done

sqlite3 "${v1}" <<SQL
PRAGMA foreign_keys = ON;
CREATE TABLE _sqlx_migrations (
    version BIGINT PRIMARY KEY,
    description TEXT NOT NULL,
    installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    success BOOLEAN NOT NULL,
    checksum BLOB NOT NULL,
    execution_time BIGINT NOT NULL
);
.read ${migration_dir}/0001_core.sql
INSERT INTO _sqlx_migrations
    (version, description, installed_on, success, checksum, execution_time)
VALUES
    (1, 'core', '2026-01-01 00:00:00', 1,
     X'2c0d9dc5f1cc1bb705dcf00abc0cd351b8aaa77c3af2543264574f490502c62572f9625b0059bd5908b9f5e4a85c1f81',
     0);

INSERT INTO provider_profile
    (id, name, dialect, base_url, default_model, created_at, updated_at)
VALUES
    ('provider-upgrade', 'Legacy provider', 'openai_chat_completions',
     'https://example.com/v1', 'legacy-model', 1, 1);
INSERT INTO workspace
    (id, title, system_prompt, created_at, updated_at)
VALUES
    ('workspace-upgrade', 'Legacy workspace', 'Legacy system prompt', 1, 1);
INSERT INTO content_block
    (id, role, content, content_hash, created_at)
VALUES
    ('prompt-upgrade', 'user', 'hello', 'prompt-hash-upgrade', 1);
INSERT INTO turn
    (id, workspace_id, parent_run_id, prompt_block_id, title, created_at)
VALUES
    ('turn-upgrade', 'workspace-upgrade', NULL, 'prompt-upgrade', '', 1);
INSERT INTO model_run
    (id, turn_id, workspace_id, provider_profile_id, model, status,
     output_markdown, reasoning_markdown, provider_snapshot_json,
     created_at, finished_at)
VALUES
    ('run-upgrade', 'turn-upgrade', 'workspace-upgrade', 'provider-upgrade',
     'legacy-model', 'completed', 'answer', '',
     '{"dialect":"openai_chat_completions"}', 1, 2);
INSERT INTO context_manifest
    (id, workspace_id, compiler_version, strategy, estimated_chars,
     canonical_hash, warnings_json, created_at)
VALUES
    ('manifest-upgrade', 'workspace-upgrade', '1',
     'ancestor_path_with_pins', 5, 'hash-upgrade', '[]', 1);
INSERT INTO context_manifest_item
    (manifest_id, workspace_id, position, source_id, source_kind, role,
     content_block_id, inclusion_reason)
VALUES
    ('manifest-upgrade', 'workspace-upgrade', 0, 'turn-upgrade',
     'current_prompt', 'user', 'prompt-upgrade', 'current_prompt');
INSERT INTO context_snapshot
    (id, run_id, manifest_id, workspace_id, provider_profile_id, provider,
     model, base_url, parameters_json, request_json, canonical_hash, created_at)
VALUES
    ('snapshot-upgrade', 'run-upgrade', 'manifest-upgrade', 'workspace-upgrade',
     'provider-upgrade', 'Legacy provider', 'legacy-model',
     'https://example.com/v1', '{}', '{}', 'hash-upgrade', 1);
INSERT INTO branch_pointer
    (id, workspace_id, name, head_run_id, version, created_at, updated_at)
VALUES
    ('branch-upgrade', 'workspace-upgrade', 'Legacy branch',
     'run-upgrade', 3, 1, 2);
VACUUM;
SQL

cp "${v1}" "${v2}"
sqlite3 "${v2}" <<SQL
PRAGMA foreign_keys = ON;
.read ${migration_dir}/0002_workspace_goal.sql
INSERT INTO _sqlx_migrations
    (version, description, installed_on, success, checksum, execution_time)
VALUES
    (2, 'workspace goal', '2026-01-02 00:00:00', 1,
     X'd0e7ffa2a330c621a834af82fef334eb1b90b629fba971f8806e03533c07533f39a898bb04a9dbeb17605c5f93c20386',
     0);
VACUUM;
SQL

cp "${v2}" "${v3}"
sqlite3 "${v3}" <<SQL
PRAGMA foreign_keys = ON;
.read ${migration_dir}/0003_provider_template.sql
INSERT INTO _sqlx_migrations
    (version, description, installed_on, success, checksum, execution_time)
VALUES
    (3, 'provider template', '2026-01-03 00:00:00', 1,
     X'812e37afed324ae3569147851a956cd6eea7aad3825611ccc80d501ed1a5fb45f312186b057fb6d84e6c5abd5dc5c8b1',
     0);
VACUUM;
SQL

cp "${v3}" "${v4}"
sqlite3 "${v4}" <<SQL
PRAGMA foreign_keys = ON;
.read ${migration_dir}/0004_provider_dialects.sql
INSERT INTO _sqlx_migrations
    (version, description, installed_on, success, checksum, execution_time)
VALUES
    (4, 'provider dialects', '2026-01-04 00:00:00', 1,
     X'bad609d79f4e981c5fbd112f7c70ed2c268da8d0bb7194ff3540a655665b094348a4193ccfa50c4c0849d80bf69be833',
     0);
VACUUM;
SQL

echo "Generated frozen SQLite fixtures in ${fixture_dir}"
