import { ApiClientError, type RequestJsonOptions } from "@xcss/web/http-client";
import type { AdministratorApiClient } from "@xcss/web/admin-web";
import { t } from "@xcss/web/admin-ui/i18n";

const queued = new WeakMap<AdministratorApiClient, Promise<unknown>>();
export const TAG_SCAN_TIMEOUT_MS = 90_000;

function retryDelay(milliseconds: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal?.aborted) {
      reject(new DOMException("The view changed.", "AbortError"));
      return;
    }
    const timer = setTimeout(() => {
      signal?.removeEventListener("abort", abort);
      resolve();
    }, milliseconds);
    function abort() {
      clearTimeout(timer);
      signal?.removeEventListener("abort", abort);
      reject(new DOMException("The view changed.", "AbortError"));
    }
    signal?.addEventListener("abort", abort, { once: true });
  });
}

/** Consume each bounded tag response before admitting the next SQLite read. */
export function tagRequest<T>(
  client: AdministratorApiClient,
  path: string,
  guard: (value: unknown) => value is T,
  method?: string,
  body?: unknown,
  signal?: AbortSignal,
): Promise<T> {
  const result = (queued.get(client) ?? Promise.resolve()).then(async () => {
    const options: RequestJsonOptions = {
      method,
      // Collection has a 30s server budget, followed by the index transaction.
      timeoutMs: path === "/scan" ? TAG_SCAN_TIMEOUT_MS : 5000,
      maxResponseBytes: 8 * 1024 * 1024,
      ...(signal ? { signal } : {}),
      ...(body === undefined ? {} : { body: JSON.stringify(body) }),
    };
    for (let attempt = 0; ; attempt++) {
      if (signal?.aborted)
        throw new DOMException("The view changed.", "AbortError");
      try {
        return await client.request(`/api/v1/file-tags${path}`, guard, options);
      } catch (cause: unknown) {
        if (cause instanceof ApiClientError && cause.status === 401)
          window.location.replace("/__xczs__/login");
        // Directory metadata and scans can briefly own the same connection.
        // Retry only explicit temporary read failures; never repeat mutations.
        if (
          (!method || method === "GET") &&
          attempt < 2 &&
          cause instanceof ApiClientError &&
          cause.retryable &&
          ((cause.status === 429 && cause.code === "too_many_requests") ||
            (cause.status === 503 && cause.code === "service_unavailable")) &&
          (cause.retryAfterSeconds ?? 1) <= 2
        ) {
          await retryDelay(Math.max(0, cause.retryAfterSeconds ?? 1) * 1000, signal);
          continue;
        }
        throw cause;
      }
    }
  });
  queued.set(
    client,
    result.catch(() => {}),
  );
  return result;
}

export function tagFailure(cause: unknown): string {
  if (cause instanceof ApiClientError && cause.code === "tag_limit_exceeded")
    return t("每个文件最多添加 16 个标签，请先移除现有标签。", "Each file can have at most 16 tags. Remove an existing tag first.");
  if (cause instanceof ApiClientError && cause.code === "capacity_exhausted")
    return t(
      "已达到持久容量限制，请先整理数据或备份文件。",
      "Storage capacity reached. Organize your data or backups before retrying.",
    );
  return t(
    "无法读取或更新标签，请稍后重试。",
    "Could not read or update tags. Please try again.",
  );
}
