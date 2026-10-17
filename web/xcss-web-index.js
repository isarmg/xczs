import "./xcss-web-platform.js";
import initialize from "./pkg/xczs_web.js";
await initialize({ module_or_path: new URL("./pkg/xczs_web_bg.wasm", import.meta.url) });
