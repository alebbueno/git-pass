#!/usr/bin/env node
// Builds the binaries bundled in GitPass.app (`bundle.externalBin`) and copies them to
// src-tauri/binaries/<sidecar>-<target triple>. `tauri build` runs it through `npm run build:app`.
//
// Target: `--target <triple>`, else TAURI_ENV_TARGET_TRIPLE (set by the Tauri CLI), else the host.
// `universal-apple-darwin` builds both macOS architectures and joins them with `lipo`.
// Debug profile with `--debug` or TAURI_ENV_DEBUG=true. Honours CARGO_TARGET_DIR.

import { execFileSync } from "node:child_process";
import { chmodSync, copyFileSync, existsSync, mkdirSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const desktopDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const workspaceDir = resolve(desktopDir, "..");
const binariesDir = join(desktopDir, "src-tauri", "binaries");

// `sidecar` must match tauri.conf.json `bundle.externalBin` and `gitpass_core::integration::SIDECARS`.
const SIDECARS = [
  { bin: "gitpass", sidecar: "gitpass-cli" },
  { bin: "git-credential-gitpass", sidecar: "gitpass-credential-helper" },
  { bin: "gitpassd", sidecar: "gitpass-daemon" },
];
const PACKAGES = ["gitpass-cli", "gitpassd"];

function capture(cmd, args) {
  return execFileSync(cmd, args, { cwd: workspaceDir, stdio: ["ignore", "pipe", "inherit"], encoding: "utf8" });
}

function argValue(name) {
  const i = process.argv.indexOf(name);
  return i >= 0 ? process.argv[i + 1] : undefined;
}

const host = /^host:\s*(\S+)/m.exec(capture("rustc", ["-vV"]))?.[1];
if (!host) throw new Error("build-sidecars: could not read the host triple from `rustc -vV`");
const target = argValue("--target") || process.env.TAURI_ENV_TARGET_TRIPLE || host;
const debug = process.argv.includes("--debug") || process.env.TAURI_ENV_DEBUG === "true";
const profileDir = debug ? "debug" : "release";
const targetDir = process.env.CARGO_TARGET_DIR
  ? resolve(process.env.CARGO_TARGET_DIR)
  : JSON.parse(capture("cargo", ["metadata", "--format-version", "1", "--no-deps"])).target_directory;

/** Builds for `triple` and returns the directory holding the binaries. */
function build(triple) {
  const args = ["build", ...PACKAGES.flatMap((p) => ["-p", p])];
  if (!debug) args.push("--release");
  // Host builds reuse target/<profile>; `--target` would rebuild everything into target/<triple>/.
  if (triple !== host) args.push("--target", triple);
  console.log(`build-sidecars: cargo ${args.join(" ")}`);
  execFileSync("cargo", args, { cwd: workspaceDir, stdio: "inherit" });
  return triple === host ? join(targetDir, profileDir) : join(targetDir, triple, profileDir);
}

function sidecarFileName(sidecar, triple) {
  const base = `${sidecar}-${triple}`;
  return triple.includes("windows") ? `${base}.exe` : base;
}

function cargoBinPath(outDir, bin, triple) {
  const name = triple.includes("windows") ? `${bin}.exe` : bin;
  return join(outDir, name);
}

function install(src, sidecar, triple) {
  const dest = join(binariesDir, sidecarFileName(sidecar, triple));
  const tmp = `${dest}.tmp`;
  copyFileSync(src, tmp);
  chmodSync(tmp, 0o755);
  renameSync(tmp, dest);
  console.log(`build-sidecars: ${dest}`);
}

mkdirSync(binariesDir, { recursive: true });
if (!existsSync(join(binariesDir, ".gitignore"))) writeFileSync(join(binariesDir, ".gitignore"), "*\n");

if (target === "universal-apple-darwin") {
  // tauri-build checks the per-arch names while compiling; the bundler takes the universal one.
  const arches = ["aarch64-apple-darwin", "x86_64-apple-darwin"];
  const outDirs = arches.map(build);
  for (const s of SIDECARS) {
    arches.forEach((t, i) => install(cargoBinPath(outDirs[i], s.bin, t), s.sidecar, t));
    const dest = join(binariesDir, sidecarFileName(s.sidecar, target));
    execFileSync("lipo", ["-create", "-output", dest, ...outDirs.map((d) => join(d, s.bin))], { stdio: "inherit" });
    chmodSync(dest, 0o755);
    console.log(`build-sidecars: ${dest}`);
  }
} else {
  const outDir = build(target);
  for (const s of SIDECARS) install(cargoBinPath(outDir, s.bin, target), s.sidecar, target);
}
