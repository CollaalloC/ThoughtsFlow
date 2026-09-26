import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import test from "node:test";

const executable = fileURLToPath(new URL("./orca-cli.mjs", import.meta.url));

test("requires opt-in and exercises coordinator, question, completion, and release without a model", () => {
  const env = { ...process.env };
  delete env.TF_AGENT_FIXTURE_STATE;
  assert.throws(() => execFileSync(process.execPath, [executable, "status", "--json"], { env }), (error) => JSON.parse(String(error.stdout)).error.code === "fixture_disabled");
  const temporary = mkdtempSync(join(tmpdir(), "tf-orca-fixture-"));
  env.TF_AGENT_FIXTURE_STATE = join(temporary, "state.json");
  const cli = (...argv) => JSON.parse(execFileSync(process.execPath, [executable, ...argv, "--json"], { env, encoding: "utf8" })).result;
  try {
    assert.equal(cli("status").runtime.runtimeId, "fixture-runtime");
    const coordinator = cli("terminal", "create", "--worktree", "current").terminal.handle;
    assert.equal(cli("orchestration", "run-current", "--from", coordinator).run, null);
    const runId = cli("orchestration", "run-create", "--objective", "Fixture verification", "--from", coordinator).run.id;
    const current = cli("orchestration", "run-current", "--from", coordinator).run;
    assert.equal(current.id, runId);
    assert.equal(current.consumer_generation, 1);
    assert.equal(current.coordinator_handle, coordinator);
    const launchArgs = ["orchestration", "worker-start", "--run", runId, "--from", coordinator, "--agent", "omp", "--spec", "Perform fixture verification", "--retry-request", "fixed-request"];
    const started = cli(...launchArgs);
    assert.equal(cli(...launchArgs).mutation.replayed, true);
    const stateBeforeRead = readFileSync(env.TF_AGENT_FIXTURE_STATE, "utf8");
    const peek = cli("orchestration", "check", "--run", runId, "--terminal", coordinator, "--peek");
    assert.equal(readFileSync(env.TF_AGENT_FIXTURE_STATE, "utf8"), stateBeforeRead);
    assert.equal(peek.messages[0].type, "question");
    assert.equal(typeof peek.messages[0].payload, "string");
    assert.throws(() => cli("orchestration", "worker-release", "--dispatch", started.dispatchId), (error) => JSON.parse(String(error.stdout)).error.code === "worker_not_settled");
    const reply = cli("orchestration", "reply", "--run", runId, "--from", coordinator, "--id", peek.messages[0].id, "--body", "继续");
    assert.equal(reply.message.run_id, runId);
    assert.equal(reply.message.thread_id, peek.messages[0].id);
    assert.equal(reply.question.answer_message_id, reply.message.id);
    assert.ok(cli("orchestration", "check", "--run", runId, "--terminal", coordinator, "--peek").messages.some((message) => message.id === peek.messages[0].id));
    const workers = cli("orchestration", "worker-list", "--run", runId).workers;
    assert.equal(workers.length, 1);
    assert.equal(workers[0].terminalState, "reclaimable");
    assert.equal(workers[0].projection.outcome, "succeeded");
    assert.equal(workers[0].resource.ownershipState, "owned");
    assert.equal(workers[0].resource.ownerDispatchId, started.dispatchId);
    const completedTask = cli("orchestration", "task-list", "--run", runId).tasks[0];
    assert.equal(completedTask.status, "completed");
    assert.equal(completedTask.dispatch_id, null);
    const output = cli("orchestration", "worker-read", "--dispatch", started.dispatchId);
    assert.equal(output.transcript.messages[0].blocks[0].text, "OMP fixture output; no model was called");
    cli("orchestration", "worker-release", "--dispatch", started.dispatchId);
    const released = cli("orchestration", "worker-list", "--run", runId).workers[0];
    assert.equal(released.terminalState, "released");
    assert.equal(released.projection.liveness.verdict, "exited");
    assert.equal(cli("orchestration", "worker-read", "--dispatch", started.dispatchId).archived, true);
    const calls = readFileSync(`${env.TF_AGENT_FIXTURE_STATE}.calls.jsonl`, "utf8").trim().split("\n").map(JSON.parse);
    assert.ok(calls.some(({ argv }) => argv.includes("worker-start")));
    assert.ok(calls.some(({ argv }) => argv.includes("--peek")));
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
});
