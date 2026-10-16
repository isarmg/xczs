import { mountLoginPage, startAfterFonts } from "./platform.ts";
import {
  administratorApi,
  authenticationErrorMessage,
} from "./modules/platform-session.ts";

const container = document.getElementById("xczs-root");
if (!container) throw new Error("Xczs application root is missing");
void startAfterFonts(() =>
  mountLoginPage(
    container,
    async (username, password) => {
      await administratorApi.login(username, password);
      location.replace("/");
    },
    authenticationErrorMessage,
  ),
);
