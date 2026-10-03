#!/usr/bin/env node
// Runs the native `prune` binary from the platform package npm installed alongside this one.
"use strict";

const { spawnSync } = require("node:child_process");

const pkg = `prunecode-${process.platform}-${process.arch}`;

function binaryPath() {
  if (process.env.PRUNE_BINARY) return process.env.PRUNE_BINARY;
  try {
    return require.resolve(`${pkg}/bin/prune`);
  } catch {
    console.error(
      `prune: no prebuilt binary for ${process.platform}-${process.arch} (looked for ${pkg}).\n` +
        "Install from source with `cargo install prunecode`, or set PRUNE_BINARY to a prune executable."
    );
    process.exit(1);
  }
}

const result = spawnSync(binaryPath(), process.argv.slice(2), { stdio: "inherit" });
if (result.error) {
  console.error(`prune: ${result.error.message}`);
  process.exit(1);
}
process.exit(result.status ?? 1);
