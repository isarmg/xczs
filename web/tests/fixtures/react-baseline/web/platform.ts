export {
  createAdministratorApiClient,
  isAdministratorPassword,
} from "@xcss/web/admin-web";
export {
  isAdministratorSession,
  isAdministratorLoginRequest,
  isErrorEnvelope,
} from "@xcss/web/contracts";
export { ApiClientError } from "@xcss/web/http-client";
import { startAfterFonts as startPrepared } from "@xcss/web/web-fonts";
import { startServerCorners } from "@xcss/web/admin-ui/server-corners";

if (typeof document !== "undefined") {
  startServerCorners(document);
}

export function startAfterFonts(start: () => void) {
  return startPrepared(start);
}
import {
  mountFileWorkspace as renderFiles,
  mountLoginPage as renderLogin,
} from "./react/application.tsx";

export function mountFileWorkspace(
  container: HTMLElement,
  client: import("@xcss/web/admin-web").AdministratorApiClient,
  directory: string,
) {
  renderFiles(container, client, directory);
}

export function mountLoginPage(
  container: HTMLElement,
  login: (username: string, password: string) => Promise<void>,
  errorMessage: (error: unknown) => string,
) {
  renderLogin(container, login, errorMessage);
}
export {
  t,
  getLocale,
  initializeLanguage,
  switchLanguage,
  languageLabel,
  createLanguageControl,
  validationMessage,
} from "@xcss/web/admin-ui/i18n";
import "@xcss/web/design-tokens/tokens.css";
import "@xcss/web/design-tokens/tokens.dark.css";
import "@xcss/web/design-tokens/reset.css";
import "@xcss/web/design-tokens/accessibility.css";
import "@xcss/web/web-fonts/fonts.css";
import "@xcss/web/admin-ui/styles.css";
export { default as fontLicenseUrl } from "@xcss/web/web-fonts/OFL.txt?url";
export { default as cjkFontLicenseUrl } from "@xcss/web/web-fonts/CJK-LICENSE.txt?url";
