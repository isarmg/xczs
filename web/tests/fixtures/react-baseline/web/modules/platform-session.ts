import { t } from "../platform.ts";
import { createAdministratorApiClient, ApiClientError } from "../platform.ts";

// One in-memory platform client per document. No credential storage in HTML or storage APIs.
export const administratorApi = createAdministratorApiClient();

export function authenticationErrorMessage(error: unknown): string {
  const requestId =
    error instanceof ApiClientError ? error.requestId : undefined;
  return (
    t(
      "无法完成认证请求，请重试。",
      "Authentication request could not be completed. Please try again.",
    ) + (requestId ? t(" 请求标识：{0}", " Request ID: {0}", [requestId]) : "")
  );
}
