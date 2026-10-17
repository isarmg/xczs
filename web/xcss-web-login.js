import "./xcss-web/vendor/xcss-web/startup.css";
import { startApplication } from "./xcss-web/vendor/xcss-web/startup.js";
await startApplication(async () => {
  await import("./xcss-web-platform.js");
  const { default: initialize } = await import("./pkg/xczs_web.js");
  await initialize({ module_or_path: new URL("./pkg/xczs_web_bg.wasm", import.meta.url) });
});
