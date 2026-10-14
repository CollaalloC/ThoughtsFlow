#!/usr/bin/env python3
"""Compare the former full history read with the production first summary page.

Run: python3 scripts/benchmark-agent-history.py
Uses real migrations in an isolated in-memory database; never opens user data.
Measures SQLite plus Python JSON decoding, not Rust, IPC, disk or desktop latency.
The page deliberately returns less data; exact complete traversal is checked
separately, outside timing. There is no elapsed-time CI threshold.
"""

import json
from pathlib import Path
import re
import sqlite3
import statistics
import time

ROOT = Path(__file__).resolve().parents[1]
PAGE_SIZE = 50
MAX_ROWID = (1 << 63) - 1
OLD_QUERIES = {
    "evidence": "SELECT body_json FROM agent_evidence WHERE workspace_id = ? ORDER BY rowid DESC",
    "handoff": "SELECT body_json FROM agent_handoff WHERE mission_id = ? ORDER BY rowid DESC",
}


def production_query(kind):
    source = (ROOT / "src-tauri/src/agents/store.rs").read_text(encoding="utf-8")
    match = re.search(
        rf"async fn {kind}_page\(.*?sqlx::query_as\(\s*\"(.*?)\"\s*,?\s*\)",
        source,
        re.DOTALL,
    )
    if match is None:
        raise RuntimeError(f"Cannot locate production {kind}_page SQL; update the benchmark reader")
    query = re.sub(r"\\\n\s*", "", match[1])
    if "\\" in query:
        raise RuntimeError("SQL gained Rust escapes; update the benchmark reader")
    return query


def seed(connection, workspace, start, stop):
    mission = f"{workspace}-mission"
    for index in range(start, stop):
        identity = f"{workspace}-{index}"
        operation = f"h-{identity}"
        handoff = {
            "operationId": operation, "workspaceId": workspace, "missionId": mission,
            "createdAt": "2026-10-06T00:00:00Z", "preview": {
                "title": f"Task {index}", "text": "y" * 32_000, "hash": f"hash-{identity}",
                "sourceRunIds": [f"context-{identity}"],
                "evidenceIds": [f"e-{workspace}-{index - 1}"] if index else [], "sources": [],
            },
        }
        evidence = {
            "id": f"e-{identity}", "workspaceId": workspace, "missionId": mission,
            "runId": f"run-{workspace}", "taskId": f"task-{identity}",
            "dispatchId": f"dispatch-{identity}", "handoffOperationId": operation,
            "text": "x" * 64_000, "contentHash": f"hash-{identity}", "hasMore": True,
            "warning": None, "source": "terminal", "requestCursor": None, "cursor": None,
            "observedTaskStatus": "running", "createdAt": "2026-10-06T00:00:00Z",
        }
        connection.execute(
            "INSERT INTO agent_operation(id,mission_id,kind,status,request_json,created_at) "
            "VALUES(?,?,'start-task','succeeded','{}','2026-10-06T00:00:00Z')",
            (operation, mission),
        )
        connection.execute("INSERT INTO agent_handoff VALUES(?,?,?,?)", (
            operation, workspace, mission, json.dumps(handoff),
        ))
        connection.execute("INSERT INTO agent_evidence VALUES(?,?,?,?,?,?)", (
            evidence["id"], workspace, mission, "{}", json.dumps(evidence), "{}",
        ))
    connection.commit()


def expected_summary(kind, body, titles):
    if kind == "evidence":
        return {**{key: value for key, value in body.items() if key != "text"},
                "title": titles.get(body["handoffOperationId"])}
    preview = body["preview"]
    return {**{key: body[key] for key in ("operationId", "workspaceId", "missionId", "createdAt")},
            **{key: preview[key] for key in ("title", "hash", "sourceRunIds", "evidenceIds")}}


def measure(connection, query, parameters, page=False):
    samples = []
    for round_index in range(9):
        started = time.perf_counter()
        rows = connection.execute(query, parameters).fetchall()
        returned_rows = rows[:PAGE_SIZE] if page else rows
        result = [json.loads(row[-1]) for row in returned_rows]
        elapsed = (time.perf_counter() - started) * 1_000
        if round_index >= 2:
            samples.append(elapsed)
    return result, {
        "rowsRead": len(rows), "rowsReturned": len(returned_rows),
        "jsonBytesRead": sum(len(row[-1].encode()) for row in rows),
        "returnedJsonBytes": sum(len(row[-1].encode()) for row in returned_rows),
        "medianMs": statistics.median(samples), "samplesMs": samples,
    }


def verify_complete_traversal(connection, query, scope, expected):
    upper = MAX_ROWID
    observed = []
    while True:
        rows = connection.execute(query, (scope, upper)).fetchall()
        assert len(rows) <= PAGE_SIZE + 1
        observed.extend(json.loads(row[1]) for row in rows[:PAGE_SIZE])
        if len(rows) <= PAGE_SIZE:
            break
        next_upper = rows[PAGE_SIZE - 1][0] - 1
        assert next_upper < upper
        upper = next_upper
    assert observed == expected, "Paged summaries changed scope, identity, fields or ordering"


def main():
    if sqlite3.sqlite_version_info < (3, 37, 0):
        raise SystemExit("SQLite 3.37+ is required by the production STRICT schema")
    queries = {kind: production_query(kind) for kind in OLD_QUERIES}
    connection = sqlite3.connect(":memory:")
    connection.execute("PRAGMA foreign_keys=ON")
    for migration in sorted((ROOT / "src-tauri/migrations").glob("*.sql")):
        connection.executescript(migration.read_text(encoding="utf-8"))
    for workspace in ("selected", "unrelated"):
        connection.execute(
            "INSERT INTO workspace(id,title,created_at,updated_at) VALUES(?,?,1,1)",
            (workspace, workspace),
        )
        connection.execute("INSERT INTO agent_mission(id,workspace_id,body_json) VALUES(?,?,'{}')",
                           (f"{workspace}-mission", workspace))
    print(json.dumps({"benchmark": "agent-history-sqlite", "sqlite": sqlite3.sqlite_version,
                      "scope": "in-memory SQLite + Python JSON decode; not Rust, IPC, disk or desktop",
                      "warmups": 2, "rounds": 7, "pageSize": PAGE_SIZE,
                      "evidenceTextBytes": 64_000, "handoffTextBytes": 32_000}))
    previous = 0
    for count in (0, 100, 500, 1_000):
        for workspace in ("selected", "unrelated"):
            seed(connection, workspace, previous, count)
        previous = count
        titles = {row["operationId"]: row["preview"]["title"] for (body,) in connection.execute(
            OLD_QUERIES["handoff"], ("selected-mission",)) for row in [json.loads(body)]}
        for kind, query in queries.items():
            scope = "selected" if kind == "evidence" else "selected-mission"
            full, before = measure(connection, OLD_QUERIES[kind], (scope,))
            expected = [expected_summary(kind, row, titles) for row in full]
            page, after = measure(connection, query, (scope, MAX_ROWID), page=True)
            assert len(full) == count and page == expected[:PAGE_SIZE]
            verify_complete_traversal(connection, query, scope, expected)
            print(json.dumps({"kind": kind, "recordsPerScope": count, "unrelatedRecords": count,
                              "beforeFullHistory": before, "currentFirstPage": after,
                              "plan": [row[3] for row in connection.execute(
                                  "EXPLAIN QUERY PLAN " + query, (scope, MAX_ROWID))]}))
    connection.close()


if __name__ == "__main__":
    main()
