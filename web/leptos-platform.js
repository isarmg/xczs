import { startServerCorners } from "@xcss/web/admin-ui/server-corners";
import { administrator_session_json, query_job_json } from "./pkg/xczs_web.js";
// Browser business logic is compiled from web/leptos; this entry only supplies
// immutable authored styles and license assets to the existing Vite pipeline.
import "@xcss/web/design-tokens/tokens.css";
import "@xcss/web/design-tokens/tokens.dark.css";
import "@xcss/web/design-tokens/reset.css";
import "@xcss/web/design-tokens/accessibility.css";
import "@xcss/web/web-fonts/fonts.css";
import "@xcss/web/admin-ui/styles.css";
import "./tags.css";
export { default as fontLicenseUrl } from "@xcss/web/web-fonts/OFL.txt?url";
export { default as cjkFontLicenseUrl } from "@xcss/web/web-fonts/CJK-LICENSE.txt?url";

// Non-React paint adapter tracks authored CSS corner geometry as nodes mount.
startServerCorners(document);

// Read-only diagnostic bridge for browser contract tests. Credentials remain
// owned by the Rust client and are never copied into HTML or persistent storage.
export function administratorSession() {
  return JSON.parse(administrator_session_json());
}

export async function queryJob(id) { return JSON.parse(await query_job_json(id)); }
