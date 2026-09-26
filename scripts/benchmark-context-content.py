#!/usr/bin/env python3
"""Compare workspace content hydration against the pre-0007 query.

Run with Python 3.9+ and SQLite 3.37+: python3 scripts/benchmark-context-content.py
Uses a temporary in-memory database, production migrations and the current Rust
query. Measures one SQL read, not the whole workspace load or desktop latency.
"""

import json
from pathlib import Path
import re
import sqlite3
import statistics
import time


ROOT = Path(__file__).resolve().parents[1]
BASELINE = """
SELECT b.id, b.role, b.content, b.content_hash, b.created_at
FROM content_block b
WHERE EXISTS (
    SELECT 1 FROM turn t WHERE t.workspace_id = ? AND t.prompt_block_id = b.id
) OR EXISTS (
    SELECT 1 FROM context_manifest_item i
    JOIN context_manifest m ON m.id = i.manifest_id
    WHERE m.workspace_id = ? AND i.content_block_id = b.id
) OR EXISTS (
    SELECT 1 FROM context_checkpoint c
    WHERE c.workspace_id = ? AND c.summary_block_id = b.id
) OR EXISTS (
    SELECT 1 FROM context_override_item o
    WHERE o.workspace_id = ? AND o.content_block_id = b.id
)
ORDER BY b.created_at, b.id
"""


def production_query():
    source = (ROOT / "src-tauri/src/infrastructure/sqlite/repository.rs").read_text(encoding="utf-8")
    match = re.search(
        r'async fn list_content_blocks_on\(.*?sqlx::query\(\s*"(.*?)",\s*\)',
        source,
        re.DOTALL,
    )
    if match is None:
        raise RuntimeError("Cannot locate the production content query; update the benchmark reader")
    # Rust's escaped newline removes the following indentation.
    return json.loads('"' + re.sub(r"\\\n\s*", "", match[1]) + '"')


def seed(connection, workspace, prefix, start, stop):
    for index in range(start, stop):
        block_id = f"{prefix}block-{index}"
        manifest_id = f"{prefix}manifest-{index}"
        connection.execute(
            "INSERT INTO content_block VALUES (?, 'user', ?, ?, ?)",
            (block_id, f"{prefix}内容 {index}", f"{prefix}hash-{index}", index),
        )
        connection.execute(
            "INSERT INTO turn (id, workspace_id, prompt_block_id, created_at) VALUES (?, ?, ?, ?)",
            (f"{prefix}turn-{index}", workspace, block_id, index),
        )
        connection.execute(
            "INSERT INTO context_manifest "
            "(id, workspace_id, compiler_version, strategy, estimated_chars, canonical_hash, created_at) "
            "VALUES (?, ?, '4', 'benchmark', 1, ?, ?)",
            (manifest_id, workspace, f"{prefix}canonical-{index}", index),
        )
        connection.execute(
            "INSERT INTO context_manifest_item "
            "(manifest_id, workspace_id, position, source_kind, role, content_block_id, inclusion_reason) "
            "VALUES (?, ?, 0, 'current_prompt', 'user', ?, 'current_prompt')",
            (manifest_id, workspace, block_id),
        )
    connection.commit()


def measure(connection, query, parameters):
    samples = []
    for index in range(9):
        start = time.perf_counter()
        rows = connection.execute(query, parameters).fetchall()
        if index >= 2:
            samples.append((time.perf_counter() - start) * 1000)
    steps = 0

    def progress():
        nonlocal steps
        steps += 100
        return 0

    connection.set_progress_handler(progress, 100)
    try:
        connection.execute(query, parameters).fetchall()
    finally:
        connection.set_progress_handler(None, 0)
    return rows, {"median_ms": round(statistics.median(samples), 4), "vm_steps_rounded": steps}


def main():
    if sqlite3.sqlite_version_info < (3, 37, 0):
        raise SystemExit(f"SQLite 3.37+ is required for STRICT tables; found {sqlite3.sqlite_version}")
    current = production_query()
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys = ON")
    for migration in sorted((ROOT / "src-tauri/migrations").glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    for workspace in ("selected", "unrelated"):
        connection.execute(
            "INSERT INTO workspace (id, title, created_at, updated_at) VALUES (?, ?, 1, 1)",
            (workspace, workspace),
        )
    seed(connection, "selected", "selected-", 0, 20)
    parameters = ("selected",) * 4
    print(json.dumps({"sqlite": sqlite3.sqlite_version, "warmups": 2, "samples": 7, "selected_blocks": 20}))
    previous = 0
    for unrelated in (0, 1000, 10000, 30000):
        seed(connection, "unrelated", "unrelated-", previous, unrelated)
        previous = unrelated
        old_rows, before = measure(connection, BASELINE, parameters)
        new_rows, after = measure(connection, current, parameters)
        assert new_rows == old_rows and len(new_rows) == 20
        print(json.dumps({"unrelated_turns_and_receipts": unrelated, "before": before, "after": after}))
    for name, query in (("before", BASELINE), ("after", current)):
        plan = [row[3] for row in connection.execute("EXPLAIN QUERY PLAN " + query, parameters)]
        print(json.dumps({"query": name, "plan": plan}))
    connection.close()


if __name__ == "__main__":
    main()
