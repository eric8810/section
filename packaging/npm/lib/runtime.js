const fs = require("node:fs");
const path = require("node:path");
const { spawnSync } = require("node:child_process");
const pkg = require("../package.json");

const TARGETS = {
  darwin: {
    arm64: "darwin-arm64",
    x64: "darwin-x64",
  },
  linux: {
    arm64: "linux-arm64",
    x64: "linux-x64",
  },
};

function platformTriple() {
  const byPlatform = TARGETS[process.platform];
  if (!byPlatform || !byPlatform[process.arch]) {
    throw new Error(`unsupported npm install target: ${process.platform}-${process.arch}`);
  }

  return byPlatform[process.arch];
}

function vendorRoot() {
  return path.join(__dirname, "..", "vendor", platformTriple());
}

function binaryPath(name) {
  return path.join(vendorRoot(), "bin", name);
}

function releaseTag() {
  return `v${pkg.version}`;
}

function releaseAssetName() {
  return `section-${pkg.version}-${platformTriple()}.tar.gz`;
}

function releaseUrl() {
  return `https://github.com/eric8810/section/releases/download/${releaseTag()}/${releaseAssetName()}`;
}

function runBinary(name) {
  const executable = binaryPath(name);
  if (!fs.existsSync(executable)) {
    console.error(
      `[section] missing ${name} binary for ${platformTriple()}. Reinstall the package after the matching release asset exists.`
    );
    process.exit(1);
  }

  const result = spawnSync(executable, process.argv.slice(2), { stdio: "inherit" });
  if (result.error) {
    throw result.error;
  }

  process.exit(result.status === null ? 1 : result.status);
}

module.exports = {
  binaryPath,
  platformTriple,
  releaseAssetName,
  releaseTag,
  releaseUrl,
  runBinary,
  vendorRoot,
};
