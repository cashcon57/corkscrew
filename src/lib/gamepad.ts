/**
 * Gamepad input handler for Steam Deck / controller navigation.
 * Uses the Web Gamepad API to map controller inputs to app navigation.
 */

export type GamepadAction =
  | "up"
  | "down"
  | "left"
  | "right"
  | "confirm"
  | "back"
  | "shoulder_left"
  | "shoulder_right"
  | "menu";

type GamepadCallback = (action: GamepadAction) => void;

// Standard Gamepad button mapping (Xbox layout)
const BUTTON_MAP: Record<number, GamepadAction> = {
  0: "confirm",        // A / Cross
  1: "back",           // B / Circle
  4: "shoulder_left",  // LB / L1
  5: "shoulder_right", // RB / R1
  9: "menu",           // Start / Options
  12: "up",            // D-pad Up
  13: "down",          // D-pad Down
  14: "left",          // D-pad Left
  15: "right",         // D-pad Right
};

// Axis deadzone to prevent drift
const AXIS_DEADZONE = 0.5;
// Repeat delay for held directions (ms)
const REPEAT_DELAY = 400;
const REPEAT_INTERVAL = 120;

export class GamepadManager {
  private animFrameId: number | null = null;
  private callback: GamepadCallback;
  private prevButtons: Record<number, boolean> = {};
  private axisState: Record<string, { active: boolean; timer: ReturnType<typeof setTimeout> | null; repeating: boolean }> = {
    "left-x-neg": { active: false, timer: null, repeating: false },
    "left-x-pos": { active: false, timer: null, repeating: false },
    "left-y-neg": { active: false, timer: null, repeating: false },
    "left-y-pos": { active: false, timer: null, repeating: false },
  };
  private _connected = false;

  constructor(callback: GamepadCallback) {
    this.callback = callback;
  }

  get connected(): boolean {
    return this._connected;
  }

  start(): void {
    window.addEventListener("gamepadconnected", this.onConnect);
    window.addEventListener("gamepaddisconnected", this.onDisconnect);

    // Check if a gamepad is already connected
    const gamepads = navigator.getGamepads();
    for (const gp of gamepads) {
      if (gp) {
        this._connected = true;
        break;
      }
    }

    this.poll();
  }

  stop(): void {
    window.removeEventListener("gamepadconnected", this.onConnect);
    window.removeEventListener("gamepaddisconnected", this.onDisconnect);
    if (this.animFrameId !== null) {
      cancelAnimationFrame(this.animFrameId);
      this.animFrameId = null;
    }
    // Clear axis timers
    for (const axis of Object.values(this.axisState)) {
      if (axis.timer) clearTimeout(axis.timer);
    }
  }

  private onConnect = (): void => {
    this._connected = true;
  };

  private onDisconnect = (): void => {
    const gamepads = navigator.getGamepads();
    this._connected = gamepads.some(gp => gp !== null);
  };

  private poll = (): void => {
    const gamepads = navigator.getGamepads();
    for (const gp of gamepads) {
      if (!gp) continue;
      this.processButtons(gp);
      this.processAxes(gp);
    }
    this.animFrameId = requestAnimationFrame(this.poll);
  };

  private processButtons(gp: Gamepad): void {
    for (const [indexStr, action] of Object.entries(BUTTON_MAP)) {
      const index = Number(indexStr);
      const pressed = gp.buttons[index]?.pressed ?? false;
      const wasPrev = this.prevButtons[index] ?? false;

      // Trigger on press (not hold)
      if (pressed && !wasPrev) {
        this.callback(action);
      }
      this.prevButtons[index] = pressed;
    }
  }

  private processAxes(gp: Gamepad): void {
    if (gp.axes.length < 2) return;

    const lx = gp.axes[0];
    const ly = gp.axes[1];

    this.handleAxis("left-x-neg", lx < -AXIS_DEADZONE, "left");
    this.handleAxis("left-x-pos", lx > AXIS_DEADZONE, "right");
    this.handleAxis("left-y-neg", ly < -AXIS_DEADZONE, "up");
    this.handleAxis("left-y-pos", ly > AXIS_DEADZONE, "down");
  }

  private handleAxis(key: string, active: boolean, action: GamepadAction): void {
    const state = this.axisState[key];
    if (!state) return;

    if (active && !state.active) {
      // Just activated
      state.active = true;
      this.callback(action);
      state.timer = setTimeout(() => {
        state.repeating = true;
        const repeat = () => {
          if (state.active) {
            this.callback(action);
            state.timer = setTimeout(repeat, REPEAT_INTERVAL);
          }
        };
        repeat();
      }, REPEAT_DELAY);
    } else if (!active && state.active) {
      // Released
      state.active = false;
      state.repeating = false;
      if (state.timer) {
        clearTimeout(state.timer);
        state.timer = null;
      }
    }
  }
}
