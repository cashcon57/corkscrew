import fs from "fs";
import path from "path";
import { spawnSync } from "child_process";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));

// tauri-plugin-webdriver embeds a W3C WebDriver server in debug builds.
// No external driver process needed — the app itself listens on port 4445.

function resolveBinaryPath(): string {
  const target = path.resolve(__dirname, "..", "src-tauri", "target", "debug");

  if (process.platform === "darwin") {
    // macOS: prefer .app bundle if it exists (full native webview)
    const appBundle = path.join(
      target,
      "bundle",
      "macos",
      "Corkscrew.app",
      "Contents",
      "MacOS",
      "corkscrew"
    );
    if (fs.existsSync(appBundle)) return appBundle;
  }

  return path.join(target, "corkscrew");
}

export const config = {
  // tauri-plugin-webdriver listens on port 4445
  hostname: "127.0.0.1",
  port: 4445,

  specs: ["./specs/*.ts"],  // game-harness/ excluded — run with --spec specs/game-harness/
  maxInstances: 1,

  capabilities: [
    {
      maxInstances: 1,
      "tauri:options": {
        application: resolveBinaryPath(),
      },
    },
  ],

  runner: "local",
  reporters: ["spec"],
  framework: "mocha",
  mochaOpts: {
    ui: "bdd",
    timeout: 120_000,
  },

  // Build the debug binary if it doesn't exist (skip in CI — CI builds separately)
  onPrepare() {
    if (process.env.CI) return;

    const binaryPath = resolveBinaryPath();
    if (fs.existsSync(binaryPath)) {
      console.log("[wdio] Using existing debug binary:", binaryPath);
      return;
    }

    console.log("[wdio] No debug binary found. Building...");
    const result = spawnSync("cargo", ["build"], {
      cwd: path.resolve(__dirname, "..", "src-tauri"),
      stdio: "inherit",
      shell: true,
    });
    if (result.status !== 0) {
      throw new Error(`cargo build failed with exit code ${result.status}`);
    }
  },
};
