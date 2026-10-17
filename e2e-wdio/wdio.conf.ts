import fs from "fs";
import os from "os";
import path from "path";
import { spawn, spawnSync, type ChildProcess } from "child_process";
import { fileURLToPath } from "url";

const __dirname = fileURLToPath(new URL(".", import.meta.url));

// tauri-plugin-webdriver embeds a W3C WebDriver server in debug builds.
// No external driver process — but nothing else launches the app either, so
// each worker spawns the binary in beforeSession and waits for the embedded
// server before WebdriverIO creates its session.
const WEBDRIVER_PORT = Number(process.env.TAURI_WEBDRIVER_PORT ?? 4445);
const STATUS_URL = `http://127.0.0.1:${WEBDRIVER_PORT}/status`;
const APP_READY_TIMEOUT_MS = 60_000;
const APP_EXIT_TIMEOUT_MS = 10_000;

// Scratch home for the app under test, shared by every spec in one run. The
// launcher creates a fresh one in onPrepare; workers inherit it via the
// environment. Without it the app opens the developer's real mods.db and config.
const SANDBOX_ENV = "CORKSCREW_E2E_SANDBOX";
const SANDBOX_PREFIX = "corkscrew-e2e-";

let appProcess: ChildProcess | undefined;
let createdSandbox: string | undefined;

/** Launcher only: always a new temp dir, never one named by the caller's environment. */
function createSandbox(): string {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), SANDBOX_PREFIX));
  seedFixtureGame(path.join(dir, "home"));
  process.env[SANDBOX_ENV] = dir;
  createdSandbox = dir;
  return dir;
}

/**
 * Workers: the sandbox the launcher created. Refuse anything that isn't a
 * mkdtemp dir of ours, so an inherited value can't aim the app (or onComplete's
 * cleanup) at real data.
 */
function sandboxDir(): string {
  const dir = process.env[SANDBOX_ENV];
  if (
    !dir ||
    path.dirname(path.resolve(dir)) !== path.resolve(os.tmpdir()) ||
    !path.basename(dir).startsWith(SANDBOX_PREFIX) ||
    !fs.existsSync(dir)
  ) {
    throw new Error(`[wdio] ${SANDBOX_ENV} is not a sandbox created by onPrepare: ${dir ?? "(unset)"}`);
  }
  return dir;
}

/**
 * Plant a minimal Skyrim SE install in a Wine prefix under the sandbox home, so
 * game-dependent specs run against a real detected game (via the app's normal
 * bottle discovery) on machines with no games, CI included. The prefix sits
 * where each platform's scanner looks: `~/.wine` on Linux, a CrossOver bottle
 * on macOS. Detection only needs the executable to exist.
 */
function seedFixtureGame(home: string): void {
  const prefix =
    process.platform === "darwin"
      ? path.join(home, "Library", "Application Support", "CrossOver", "Bottles", "E2E")
      : path.join(home, ".wine");
  const gameDir = path.join(
    prefix, "drive_c", "Program Files (x86)", "Steam", "steamapps", "common", "Skyrim Special Edition"
  );
  fs.mkdirSync(path.join(gameDir, "Data"), { recursive: true });
  fs.writeFileSync(path.join(gameDir, "SkyrimSE.exe"), "e2e fixture");
}

/** App environment confined to the sandbox. */
function sandboxedEnv(): NodeJS.ProcessEnv {
  const sandbox = sandboxDir();
  const home = path.join(sandbox, "home");
  return {
    ...process.env,
    // Bottle discovery scans $HOME: the app sees only the fixture game, never real installs.
    HOME: home,
    // Always inside the sandbox, even if the caller exported CORKSCREW_DATA_DIR:
    // the mod lifecycle specs install, deploy and purge for real.
    CORKSCREW_DATA_DIR: path.join(sandbox, "corkscrew"),
    // Linux: also covers paths resolved via `dirs` directly and WebKitGTK storage.
    XDG_DATA_HOME: path.join(home, ".local", "share"),
    XDG_CONFIG_HOME: path.join(home, ".config"),
    XDG_CACHE_HOME: path.join(home, ".cache"),
  };
}

/**
 * The plain debug executable that `tauri build --debug --no-bundle` writes, on
 * every platform. Never a `bundle/macos/Corkscrew.app`: no-bundle builds don't
 * refresh it, so any .app found there is a leftover from an older build.
 */
function resolveBinaryPath(): string {
  const srcTauri = path.resolve(__dirname, "..", "src-tauri");
  const targetDir = process.env.CARGO_TARGET_DIR
    ? path.resolve(srcTauri, process.env.CARGO_TARGET_DIR)
    : path.join(srcTauri, "target");
  const exe = process.platform === "win32" ? "corkscrew.exe" : "corkscrew";
  return path.join(targetDir, "debug", exe);
}

async function webdriverReady(): Promise<boolean> {
  try {
    const res = await fetch(STATUS_URL, { signal: AbortSignal.timeout(1_000) });
    return res.ok;
  } catch {
    return false; // not listening yet
  }
}

async function launchApp(binaryPath: string): Promise<void> {
  if (!fs.existsSync(binaryPath)) {
    throw new Error(`[wdio] App binary not found: ${binaryPath}`);
  }
  // A stale instance would silently receive our session instead of the build under test.
  if (await webdriverReady()) {
    throw new Error(`[wdio] Port ${WEBDRIVER_PORT} is already serving WebDriver; close the running app first`);
  }

  const child = spawn(binaryPath, [], { stdio: "inherit", env: sandboxedEnv() });
  appProcess = child;
  // If the worker dies without reaching afterSession, don't leave the app holding the port.
  process.once("exit", () => child.kill("SIGKILL"));
  let spawnError: Error | undefined;
  child.once("error", (err) => (spawnError = err));

  const deadline = Date.now() + APP_READY_TIMEOUT_MS;
  while (Date.now() < deadline) {
    if (spawnError) throw new Error(`[wdio] Failed to start ${binaryPath}: ${spawnError.message}`);
    if (child.exitCode !== null || child.signalCode !== null) {
      throw new Error(`[wdio] App exited before WebDriver was ready (code ${child.exitCode}, signal ${child.signalCode})`);
    }
    if (await webdriverReady()) return;
    await new Promise((r) => setTimeout(r, 250));
  }
  await stopApp();
  throw new Error(`[wdio] WebDriver server did not respond at ${STATUS_URL} within ${APP_READY_TIMEOUT_MS}ms`);
}

async function stopApp(): Promise<void> {
  const child = appProcess;
  appProcess = undefined;
  if (!child || child.exitCode !== null || child.signalCode !== null) return;

  const exited = new Promise<void>((resolve) => child.once("exit", () => resolve()));
  child.kill("SIGTERM");
  const timer = setTimeout(() => child.kill("SIGKILL"), APP_EXIT_TIMEOUT_MS);
  await exited; // the next spec's app must be able to bind the port
  clearTimeout(timer);
}

export const config = {
  // tauri-plugin-webdriver listens on port 4445 (TAURI_WEBDRIVER_PORT overrides)
  hostname: "127.0.0.1",
  port: WEBDRIVER_PORT,

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

  async beforeSession() {
    await launchApp(resolveBinaryPath());
  },

  async afterSession() {
    await stopApp();
  },

  // Build the debug binary if it doesn't exist (skip in CI — CI builds separately)
  onPrepare() {
    console.log("[wdio] App sandbox:", createSandbox());
    if (process.env.CI) return;

    const binaryPath = resolveBinaryPath();
    if (fs.existsSync(binaryPath)) {
      console.log("[wdio] Using existing debug binary:", binaryPath);
      return;
    }

    console.log("[wdio] No debug binary found. Building...");
    // `tauri build --debug` embeds the built frontend; a plain `cargo build`
    // would load devUrl (localhost:1420), which isn't running here.
    const result = spawnSync("npm", ["run", "tauri", "build", "--", "--debug", "--no-bundle"], {
      cwd: path.resolve(__dirname, ".."),
      stdio: "inherit",
      shell: true,
    });
    if (result.status !== 0) {
      throw new Error(`tauri build failed with exit code ${result.status}`);
    }
  },

  onComplete() {
    if (createdSandbox) fs.rmSync(createdSandbox, { recursive: true, force: true });
  },
};
