import * as EditBufferNotifications from "./editbuffer.js";
import * as Event from "./event.js";
import * as Fig from "@aws/amazon-q-developer-cli-proto/fig";
import * as fs from "./filesystem.js";
import * as History from "./history.js";
import * as Install from "./install.js";
import * as Internal from "./requests.js";
import * as Keybindings from "./keybindings.js";
import * as Native from "./native.js";
import * as Platform from "./platform.js";
import * as Process from "./process.js";
import * as Settings from "./settings.js";
import * as Shell from "./shell.js";
import * as State from "./state.js";
import * as Telemetry from "./telemetry.js";
import * as Types from "./types.js";
import * as WindowPosition from "./position.js";

const lib = {
  EditBufferNotifications,
  Event,
  fs,
  History,
  Install,
  Internal,
  Keybindings,
  Native,
  Platform,
  Process,
  Settings,
  Shell,
  State,
  Telemetry,
  Types,
  WindowPosition,
};

export {
  EditBufferNotifications,
  Event,
  Fig,
  fs,
  History,
  Install,
  Internal,
  Keybindings,
  Native,
  Platform,
  Process,
  Settings,
  Shell,
  State,
  Telemetry,
  Types,
  WindowPosition,
};

declare global {
  interface Window {
    f: typeof lib;
  }
}

window.f = lib;
