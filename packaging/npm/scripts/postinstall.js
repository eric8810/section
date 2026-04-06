#!/usr/bin/env node

const fs = require("node:fs");
const os = require("node:os");
const path = require("node:path");
const { Readable } = require("node:stream");
const { pipeline } = require("node:stream/promises");
const { execFileSync } = require("node:child_process");
const runtime = require("../lib/runtime");

async function downloadArchive(destination) {
  const response = await fetch(runtime.releaseUrl());
  if (!response.ok || !response.body) {
    throw new Error(`failed to download ${runtime.releaseUrl()}`);
  }

  await pipeline(Readable.fromWeb(response.body), fs.createWriteStream(destination));
}

async function main() {
  const tempDir = fs.mkdtempSync(path.join(os.tmpdir(), "section-npm-"));
  const archivePath = path.join(tempDir, runtime.releaseAssetName());
  const installRoot = runtime.vendorRoot();

  console.log(`[section] downloading ${runtime.releaseAssetName()}`);
  await downloadArchive(archivePath);

  fs.rmSync(installRoot, { recursive: true, force: true });
  fs.mkdirSync(installRoot, { recursive: true });
  execFileSync("tar", ["-xzf", archivePath, "-C", installRoot], { stdio: "inherit" });

  fs.chmodSync(runtime.binaryPath("section"), 0o755);
  fs.chmodSync(runtime.binaryPath("sectiond"), 0o755);
  fs.rmSync(tempDir, { recursive: true, force: true });
}

main().catch((error) => {
  console.error(`[section] npm install failed: ${error.message}`);
  process.exit(1);
});
