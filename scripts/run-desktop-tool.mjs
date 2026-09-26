import { executeDesktopCommand, planDesktopCommand } from "./desktop-toolchain.ts";

try {
  process.exitCode = executeDesktopCommand(
    planDesktopCommand(process.argv[2], { args: process.argv.slice(3) }),
  );
} catch (error) {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
}
