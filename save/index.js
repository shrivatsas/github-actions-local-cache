"use strict";

const { spawnSync } = require("node:child_process");
const { appendFileSync } = require("node:fs");
const { join } = require("node:path");

if (process.platform !== "linux" || !["x64", "arm64"].includes(process.arch)) {
  process.stderr.write('::error::{"event":"local-cache.error","code":"unsupported-platform"}\n');
  process.exit(1);
}
const binary = join(__dirname, "dist", `local-cache-linux-${process.arch}`);
const result = spawnSync(binary, ["save"], { stdio: "inherit", shell: false });
if (result.error) {
  const fail = process.env["INPUT_FAIL-ON-CACHE-ERROR"] ?? process.env.INPUT_FAIL_ON_CACHE_ERROR ?? "true";
  if (fail === "false" && process.env.GITHUB_OUTPUT) {
    process.stderr.write('::warning::{"event":"local-cache.error","code":"launcher-error"}\n');
    appendFileSync(process.env.GITHUB_OUTPUT, "cache-save=error\n");
    process.exit(0);
  }
  process.stderr.write('::error::{"event":"local-cache.error","code":"launcher-error"}\n');
  process.exit(1);
}
process.exit(result.status ?? 1);
