import { spawnSync } from "node:child_process";
import { homedir } from "node:os";
import { join } from "node:path";

const cargoName = process.platform === "win32" ? "cargo.exe" : "cargo";
const candidates = [
  process.env.CARGO,
  cargoName,
  join(homedir(), ".cargo", "bin", cargoName),
].filter(Boolean);
const args = [
  "test",
  "--manifest-path",
  "src-tauri/Cargo.toml",
  "--test",
  "native_context_tree",
  "--",
  "--test-threads=1",
];

for (const cargo of candidates) {
  const result = spawnSync(cargo, args, { stdio: "inherit" });
  if (result.error?.code === "ENOENT") continue;
  if (result.error) throw result.error;
  process.exitCode = result.status ?? 1;
  break;
}

if (process.exitCode === undefined) {
  console.error("Rust Cargo was not found. Set CARGO or add cargo to PATH before running npm run test:native.");
  process.exitCode = 1;
}
