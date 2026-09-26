#!/usr/bin/env node
// Local integration fixture only. This executable never launches Orca, OMP, or Git.
import { appendFileSync, existsSync, mkdirSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import { dirname, isAbsolute, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout } from "node:timers/promises";

const argv = process.argv.slice(2);
const statePath = process.env.TF_AGENT_FIXTURE_STATE;
const runtimeId = "fixture-runtime";
const version = "1.4.206-fixture";
const repoPath = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
const workspaceId = `fixture-repo::${repoPath}`;
const fixtureMarker = "ThoughsFlow Orca CLI fixture";
const outputText = "OMP fixture output; no model was called";
const flag = (name) => {
  const index = argv.indexOf(`--${name}`);
  return index < 0 ? undefined : argv[index + 1];
};
const has = (name) => argv.includes(`--${name}`);
const firstFlag = argv.findIndex((item) => item.startsWith("--"));
const command = argv.slice(0, firstFlag < 0 ? undefined : firstFlag).join(" ");
const fail = (code, message) => { throw Object.assign(new Error(message), { code }); };
const required = (name) => flag(name) || fail("invalid_argument", `Missing --${name}`);
const envelope = (result) => ({ id: "fixture-response", ok: true, result, _meta: { runtimeId } });
const emit = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
const now = () => new Date().toISOString();
const isMutation = ["terminal create", "orchestration run-create", "orchestration run-use", "orchestration worker-start", "orchestration reply", "orchestration worker-release"].includes(command)
  || command === "orchestration check" && (!has("peek") && !has("all") || has("ack"));
let locked = false;

try {
  if (!statePath || !isAbsolute(statePath)) fail("fixture_disabled", "Set TF_AGENT_FIXTURE_STATE to an absolute temporary JSON path to enable this fixture.");
  mkdirSync(dirname(statePath), { recursive: true });
  // Append-only calls remain complete when the desktop polls several read endpoints together.
  const callsPath = `${statePath}.calls.jsonl`;
  appendFileSync(callsPath, `${JSON.stringify({ argv, at: now() })}\n`);
  if (argv.length === 1 && argv[0] === "--version") {
    process.stdout.write(`${version}\n`);
  } else {
    if (isMutation) {
      const deadline = Date.now() + 5_000;
      while (!locked) {
        try { mkdirSync(`${statePath}.lock`); locked = true; }
        catch (error) {
          if (error.code !== "EEXIST" || Date.now() >= deadline) throw error;
          await setTimeout(10);
        }
      }
    }
    const state = existsSync(statePath) ? JSON.parse(readFileSync(statePath, "utf8")) : {
      fixture: fixtureMarker, counter: 0, terminals: [], runs: [], tasks: [], workers: [], messages: [], deliveries: [], receipts: {}, calls: [],
    };
    if (state.fixture !== fixtureMarker) fail("fixture_state_invalid", "Refusing to overwrite a file that is not fixture state.");
    const nextId = (type) => `${type}_fixture_${++state.counter}`;
    const find = (collection, id) => state[collection].find((item) => (item.id ?? item.dispatchId ?? item.handle) === id)
      || fail("not_found", `${collection}: ${id} not found`);
    const terminal = () => find("terminals", required("terminal"));
    const run = () => find("runs", flag("run") ?? flag("id"));
    const coordinator = (runId) => {
      const handle = flag("from") ?? flag("terminal");
      find("terminals", handle);
      const selected = find("runs", runId);
      if (selected.coordinator_handle !== handle) fail("consumer_fenced", "Coordinator is not bound to this Run.");
      return selected;
    };
    const message = ({ runId, from, to, type, subject, body, payload = null, thread = null }) => {
      const row = { id: nextId("msg"), run_id: runId, from_handle: from, to_handle: to, type, subject, body,
        payload: payload === null ? null : JSON.stringify(payload), thread_id: thread, priority: "normal",
        delivery_contract: "current_delivery", created_at: now(), delivered_at: null, read: false };
      state.messages.push(row);
      return row;
    };
    const publicMessage = ({ read, ...row }) => row;
    const workerRow = (worker) => {
      const active = worker.workerState === "ready";
      const released = worker.terminalState === "released";
      return {
        ...worker,
        projection: {
          id: worker.dispatchId, dispatchId: worker.dispatchId, taskId: worker.taskId, runId: worker.runId, role: "worker", parent: null,
          provider: { id: "omp", model: "fixture-no-model" }, host: { kind: "local", id: "local" },
          workspace: { id: worker.worktreeId, kind: "folder_or_worktree" },
          stage: { worker: worker.workerState, dispatch: worker.dispatchStatus, detail: released ? "released" : active ? "input_accepted" : "settled", activity: active ? "waiting" : "idle" },
          outcome: active ? "in_progress" : "succeeded",
          liveness: { verdict: released ? "exited" : "live", source: released ? "resource_release" : "agent_status" },
          attention: { categories: active ? ["input"] : ["root_completion"], requiresAction: active },
          resource: { state: released ? "released" : "owned" },
          nextAction: { kind: worker.terminalState === "reclaimable" ? "release" : "none", argv: worker.terminalState === "reclaimable" ? ["orchestration", "worker-release", "--dispatch", worker.dispatchId] : [] },
        },
      };
    };
    const requestId = flag("retry-request") ?? (isMutation ? nextId("request") : undefined);
    const fingerprint = JSON.stringify(argv.filter((value, index) => value !== "--retry-request" && argv[index - 1] !== "--retry-request"));
    const replay = requestId && state.receipts[requestId];
    let result;
    if (replay) {
      if (replay.fingerprint !== fingerprint) fail("request_mismatch", "Request ID was used for different input.");
      result = { ...replay.result, mutation: { requestId, replayed: true } };
    } else {
      switch (command) {
        case "status":
          result = { target: { kind: "local" }, app: { running: true, pid: 12345 }, runtime: { state: "ready", reachable: true, runtimeId, appVersion: version, capabilities: ["orchestration.contract.v1"] }, graph: { state: "ready" } };
          break;
        case "repo list":
          result = { repos: [{ id: "fixture-repo", name: "ThoughsFlow", path: repoPath, kind: "git", baseRef: "main", defaultBranch: "main" }] };
          break;
        case "terminal create": {
          const row = { handle: nextId("term"), tabId: nextId("tab"), leafId: nextId("leaf"), ptyId: nextId("pty"), worktreeId: workspaceId, worktreePath: repoPath, title: flag("title") ?? "Fixture coordinator", connected: true, writable: true, surface: "terminal" };
          state.terminals.push(row);
          result = { terminal: row };
          break;
        }
        case "terminal show": result = { terminal: terminal() }; break;
        case "terminal list": result = { terminals: state.terminals, totalCount: state.terminals.length, truncated: false }; break;
        case "orchestration run-create": {
          const handle = required("from");
          find("terminals", handle);
          for (const other of state.runs) if (other.coordinator_handle === handle) other.coordinator_handle = null;
          const row = { id: nextId("run"), objective: required("objective"), coordinator_handle: handle, consumer_generation: 1, legacy: 0, created_at: now(), updated_at: now() };
          state.runs.push(row);
          result = { run: row };
          break;
        }
        case "orchestration run-show": result = { run: run() }; break;
        case "orchestration run-current": {
          const handle = required("from");
          find("terminals", handle);
          result = { run: state.runs.find((item) => item.coordinator_handle === handle) ?? null };
          break;
        }
        case "orchestration run-use": {
          const row = run();
          const handle = required("from");
          find("terminals", handle);
          for (const other of state.runs) if (other.id !== row.id && other.coordinator_handle === handle) other.coordinator_handle = null;
          if (row.coordinator_handle !== handle) { row.consumer_generation++; row.coordinator_handle = handle; state.deliveries = state.deliveries.filter((item) => item.runId !== row.id); }
          result = { run: row };
          break;
        }
        case "orchestration worker-start": {
          const selected = coordinator(required("run"));
          if (required("agent") !== "omp") fail("invalid_argument", "Fixture only supports OMP workers.");
          const taskId = nextId("task");
          const dispatchId = nextId("dispatch");
          const worker = { dispatchId, taskId, runId: selected.id, workerState: "ready", dispatchStatus: "dispatched", agentTerminalHandle: nextId("worker"), terminalState: "active", worktreeId: workspaceId, resource: { ownerDispatchId: dispatchId, ownershipState: "owned", releaseState: "active" } };
          state.workers.push(worker);
          state.tasks.push({ id: taskId, run_id: selected.id, spec: required("spec"), task_title: flag("task-title") ?? "OMP fixture task", display_name: null, status: "dispatched", assignee_handle: worker.agentTerminalHandle, dispatch_id: dispatchId, deps: "[]", result: null, created_at: now(), updated_at: now() });
          message({ runId: selected.id, from: worker.agentTerminalHandle, to: `run:${selected.id}`, type: "question", subject: "OMP fixture question", body: "是否继续执行这项验证任务？", payload: { taskId, dispatchId, options: ["继续", "说明要求"] } });
          result = { runId: selected.id, taskId, dispatchId, state: "ready", stage: "input_accepted", turnStart: "observed", setup: { state: "not_applicable" }, effects: [], residualResources: [] };
          break;
        }
        case "orchestration task-list": {
          const tasks = state.tasks.filter((item) => !flag("run") || item.run_id === flag("run"));
          result = { runId: flag("run"), tasks, count: tasks.length };
          break;
        }
        case "orchestration worker-list": {
          const workers = state.workers.filter((item) => (!flag("run") || item.runId === flag("run")) && (!flag("terminal-state") || item.terminalState === flag("terminal-state"))).toReversed();
          const offset = flag("cursor") ? Number(flag("cursor").replace("fixture-page-", "")) : 0;
          const limit = Number(flag("limit") ?? 100);
          const hasMore = offset + limit < workers.length;
          result = { workers: workers.slice(offset, offset + limit).map(workerRow), counts: Object.fromEntries(["active", "reclaimable", "released"].map((key) => [key, workers.filter((item) => item.terminalState === key).length])), page: { limit, total: workers.length, hasMore, nextCursor: hasMore ? `fixture-page-${offset + limit}` : null }, scope: { source: flag("run") ? "flag" : "all", run: flag("run") ?? null } };
          break;
        }
        case "orchestration check": {
          const selected = coordinator(required("run"));
          const prior = state.deliveries.find((item) => item.id === flag("ack") && item.runId === selected.id);
          if (has("ack") && !prior) fail("delivery_not_found", "Unknown delivery to acknowledge.");
          if (prior) { for (const item of state.messages) if (prior.messageIds.includes(item.id)) item.read = true; state.deliveries = state.deliveries.filter((item) => item !== prior); }
          const unread = state.messages.filter((item) => item.run_id === selected.id && item.to_handle === `run:${selected.id}` && (has("all") || !item.read));
          let delivery = state.deliveries.find((item) => item.runId === selected.id);
          const replayed = Boolean(delivery);
          if (!has("peek") && !has("all") && !delivery && unread.length) {
            delivery = { id: nextId("delivery"), runId: selected.id, messageIds: unread.slice(0, 50).map((item) => item.id) };
            state.deliveries.push(delivery);
          }
          const messages = delivery && !has("peek") && !has("all") ? state.messages.filter((item) => delivery.messageIds.includes(item.id)) : unread;
          result = { runId: selected.id, messages: messages.map(publicMessage), count: messages.length, acknowledged: prior?.id ?? null, ...!has("peek") && !has("all") ? { deliveryId: delivery?.id ?? null, replayed, timedOut: false, cancelled: false, connectionLost: false } : {} };
          break;
        }
        case "orchestration reply": {
          const selected = coordinator(required("run"));
          const question = find("messages", required("id"));
          if (question.type !== "question" || question.run_id !== selected.id) fail("invalid_argument", "Expected a question in this Run.");
          const { taskId, dispatchId } = JSON.parse(question.payload);
          const worker = find("workers", dispatchId);
          const task = find("tasks", taskId);
          const answer = message({ runId: selected.id, from: selected.coordinator_handle, to: `dispatch:${dispatchId}`, type: "status", subject: `Re: ${question.subject}`, body: required("body"), thread: question.id });
          // Answering does not acknowledge the coordinator's original FIFO message.
          answer.read = true;
          task.status = "completed";
          // Orca task-list joins only active dispatch contexts; settled attempts no longer fill this field.
          task.dispatch_id = null;
          task.result = outputText;
          worker.workerState = "succeeded";
          worker.dispatchStatus = "completed";
          worker.terminalState = "reclaimable";
          message({ runId: selected.id, from: worker.agentTerminalHandle, to: `run:${selected.id}`, type: "worker_done", subject: "OMP fixture completed", body: outputText, payload: { taskId, dispatchId, outcome: "succeeded", filesModified: [] } });
          result = { message: publicMessage(answer), question: { message_id: question.id, answer_message_id: answer.id, run_id: selected.id, status: "answered" }, duplicate: false };
          break;
        }
        case "orchestration worker-read": {
          const worker = find("workers", required("dispatch"));
          result = { dispatchId: worker.dispatchId, source: "transcript", provider: "omp", transcript: { messages: [{ id: "fixture-output", role: "assistant", blocks: [{ type: "text", text: outputText }] }], returnedMessageCount: 1, limited: false }, cursor: "fixture-transcript-eof", status: { worker: worker.workerState, terminal: worker.terminalState === "released" ? "exited" : "running", liveness: worker.terminalState === "released" ? "exited" : "live" }, sourceExact: true, contentComplete: true, archived: worker.terminalState === "released", fallbackReason: null, warnings: [] };
          break;
        }
        case "orchestration worker-release": {
          const worker = find("workers", required("dispatch"));
          if (worker.workerState !== "succeeded") fail("worker_not_settled", "Only a settled worker can be released.");
          const already = worker.terminalState === "released";
          worker.terminalState = "released";
          worker.resource.releaseState = "released";
          result = { dispatchId: worker.dispatchId, state: already ? "already_released" : "released", processAction: already ? "none" : "closed", archive: { source: "transcript", status: "available" } };
          break;
        }
        default: fail("unsupported_fixture_command", `Fixture does not implement: ${command}`);
      }
      if (isMutation) {
        result = { ...result, mutation: { requestId, replayed: false } };
        state.receipts[requestId] = { fingerprint, result: structuredClone(result) };
      }
    }
    if (isMutation) {
      // Read-only calls update the append-only ledger, never the shared state snapshot.
      state.calls = readFileSync(callsPath, "utf8").trim().split("\n").map((line) => JSON.parse(line));
      const temporary = `${statePath}.${process.pid}.tmp`;
      writeFileSync(temporary, JSON.stringify(state, null, 2));
      renameSync(temporary, statePath);
    }
    emit(envelope(result));
  }
} catch (error) {
  emit({ id: "fixture-response", ok: false, error: { code: error.code ?? "fixture_error", message: error.message }, _meta: { runtimeId } });
  process.exitCode = 1;
} finally {
  if (locked) rmSync(`${statePath}.lock`, { recursive: true });
}
