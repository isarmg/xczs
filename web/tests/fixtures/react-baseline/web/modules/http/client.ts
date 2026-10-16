export type OperationMetadata = {
  id: string;
  state: string;
  httpStatus: number;
};

export type UploadMetadata = {
  id: string;
  state: string;
  length: number | null;
  offset: number | null;
};

export type BoundUploadProtocol = {
  uploadId: string;
  state: string;
  length: number | null;
  offset: number | null;
};

export type ParsedErrorPayload = {
  type: string;
  title: string;
  status: number;
  detail: string;
  code: string;
  message: string;
  recovery: string;
  retryAfter: number | null;
  operation: Readonly<OperationMetadata> | null;
  upload: Readonly<UploadMetadata> | null;
};

export type RequestErrorOptions = {
  status?: number;
  problemStatus?: number;
  code?: string;
  kind?: string;
  type?: string;
  title?: string;
  detail?: string;
  recovery?: string;
  retryAfter?: number | null;
  outcomeUnknown?: boolean;
  operationId?: string;
  operationState?: string;
  operationHttpStatus?: number;
  operation?: Readonly<OperationMetadata> | null;
  uploadId?: string;
  uploadState?: string;
  uploadLength?: number | null;
  uploadOffset?: number | null;
  upload?: Readonly<UploadMetadata> | null;
  sourceRevision?: string;
  targetRevision?: string;
};

export type RequestOptions = {
  timeoutMs?: number;
  timeoutMessage?: string;
  outcomeUnknown?: boolean;
  operationId?: string;
  resultId?: string;
  limit?: number;
};

export type RequestJsonResult = {
  response: Response;
  payload: unknown;
};

export type JobStatusFields = {
  jobId: string;
  status: number;
  code: string;
  message: string;
};

export type JobState = "running" | "succeeded" | "failed" | "unknown";

export type JobQueryResult =
  | (JobStatusFields & {
      kind: "running";
      state: "running";
      authenticationFailed: false;
    })
  | (JobStatusFields & {
      kind: "succeeded";
      state: "succeeded";
      authenticationFailed: false;
    })
  | (JobStatusFields & {
      kind: "failed";
      state: "failed";
      authenticationFailed: false;
    })
  | (JobStatusFields & {
      kind: "unknown";
      state: "unknown";
      authenticationFailed: false;
    })
  | (JobStatusFields & {
      kind: "authentication";
      state: "unknown";
      authenticationFailed: true;
    })
  | (JobStatusFields & {
      kind: "unavailable";
      state: "unknown";
      authenticationFailed: false;
    });

export type MutationResult =
  | {
      kind: "succeeded";
      response: Response | null;
      status: JobQueryResult | null;
      reconciled: boolean;
    }
  | { kind: "authentication" }
  | {
      kind: "failed";
      error: unknown;
      status: JobQueryResult | null;
    };

export type UploadQueryResult = {
  operationId: string;
  state: string;
  status: number;
  code: string;
  message: string;
  authenticationFailed: boolean;
};

import { t, getLocale } from "../../platform.ts";
import {
  TARGET_REVISION_HEADER,
  UPLOAD_ID_HEADER,
  classifyUploadResponse,
} from "../upload/protocol.ts";
import {
  OPERATION_ID_HEADER,
  OPERATION_STATE_HEADER,
  classifyOperationResponse,
} from "../operations/protocol.ts";
import { platformErrorCode } from "./platform-error.ts";
import {
  ERROR_RESPONSE_BODY_LIMIT,
  SUCCESS_RESPONSE_BODY_LIMIT,
  bufferResponse as bufferBoundedResponse,
} from "./response_buffer.ts";

export { ERROR_RESPONSE_BODY_LIMIT, SUCCESS_RESPONSE_BODY_LIMIT };

export const CSRF_HEADER = "X-CSRF-Token";
export const AUTH_REQUIRED_MESSAGE = t(
  "会话已失效，正在返回登录页面。",
  "Your session is no longer valid. Returning to the sign-in page.",
);
export const PAGE_EXPIRED_MESSAGE = t(
  "会话或页面已失效。请刷新页面后重新选择文件。",
  "Your session or this page is no longer valid. Refresh the page and select the files again.",
);
export const RESULT_UNKNOWN_MESSAGE = t(
  "结果不确定，请刷新文件夹核对实际状态后再重试。",
  "The result is unknown; refresh the folder to verify what happened before trying again.",
);
export const REQUEST_TIMEOUT_MS = 30 * 1000;
export { OPERATION_ID_HEADER };
export const SOURCE_REVISION_HEADER = "X-Xczs-Source-Revision";
export { OPERATION_STATE_HEADER };

const ERROR_MESSAGE_LIMIT = 1024;
const ERROR_TYPE_LIMIT = 2048;
const ERROR_CODE_PATTERN = /^[a-z][a-z0-9_]{0,63}$/;
const ERROR_STATE_PATTERN = /^[a-z][a-z0-9-]{0,63}$/;
const RECOVERY_VALUES = new Set([
  "retry",
  "retry_with_new_id",
  "resume_upload",
  "query_job",
  "query_upload",
  "refresh_target",
]);
const OPERATION_ID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;

/** The validated subset of the upload response protocol used by this module.
 */

export class RequestError extends Error {
  status: number;
  problemStatus: number;
  code: string;
  kind: string;
  type: string;
  title: string;
  detail: string;
  recovery: string;
  retryAfter: number | null;
  outcomeUnknown: boolean;
  operationId: string;
  operationState: string;
  operationHttpStatus: number;
  operation: Readonly<OperationMetadata> | null;
  uploadId: string;
  uploadState: string;
  uploadLength: number | null;
  uploadOffset: number | null;
  upload: Readonly<UploadMetadata> | null;
  sourceRevision: string;
  targetRevision: string;
  constructor(message: string, options: RequestErrorOptions = {}) {
    super(message);
    this.name = "RequestError";
    this.status = options.status || 0;
    this.problemStatus = options.problemStatus || 0;
    this.code = options.code || "";
    this.kind = options.kind || "request";
    this.type = options.type || "";
    this.title = options.title || "";
    this.detail = options.detail || "";
    this.recovery = options.recovery || "";
    this.retryAfter = options.retryAfter ?? null;
    this.outcomeUnknown = Boolean(options.outcomeUnknown);
    this.operationId = options.operationId || "";
    this.operationState = options.operationState || "";
    this.operationHttpStatus = options.operationHttpStatus || 0;
    this.operation = options.operation || null;
    this.uploadId = options.uploadId || "";
    this.uploadState = options.uploadState || "";
    this.uploadLength = options.uploadLength ?? null;
    this.uploadOffset = options.uploadOffset ?? null;
    this.upload = options.upload || null;
    this.sourceRevision = options.sourceRevision || "";
    this.targetRevision = options.targetRevision || "";
  }
}

export async function bufferResponse(
  response: Response,
  method: string = "GET",
  options: {
    limit?: number;
    outcomeUnknown?: boolean;
    operationId?: string;
  } = {},
): Promise<Response> {
  if (response.status === 401) {
    cancelUnreadResponseBody(response.body);
    return response;
  }
  return await bufferBoundedResponse(
    response,
    method,
    options,
    (message, errorOptions) => new RequestError(message, errorOptions),
  );
}

/** Authentication is authoritative from the status and response headers. Do
 * not wait for, or apply the ordinary error-body limit to, a body that no
 * caller will inspect after `assertResponse` classifies the response.
 *
 * Cancellation is intentionally fire-and-forget: a broken response stream
 * must not be able to delay session recovery by returning a pending or
 * rejected cancellation promise.
 */
function cancelUnreadResponseBody(body: ReadableStream<Uint8Array> | null) {
  try {
    void body?.cancel().catch(() => {});
  } catch {
    // The authentication result remains authoritative even if cancellation
    // fails after the response headers have arrived.
  }
}

class AuthenticationError extends RequestError {
  constructor(message: string, options?: RequestErrorOptions) {
    super(message, options);
    this.name = "AuthenticationError";
  }
}

export function isCsrfAuthFailure(status: number, errorCode: string | null) {
  return status === 403 && errorCode === "auth.csrf_rejected";
}

const platformResponseErrors: WeakMap<Response, string | null> = new WeakMap();

/** Platform errors precede business operation registration. Bound the body before decoding.
 */
export async function responsePlatformErrorCode(
  response: Response,
): Promise<string | null> {
  if (response.status !== 403) return null;
  if (platformResponseErrors.has(response))
    return platformResponseErrors.get(response) ?? null;
  let buffered;
  try {
    buffered = await bufferBoundedResponse(
      response.clone(),
      "GET",
      { limit: ERROR_RESPONSE_BODY_LIMIT },
      (message, options) =>
        new RequestError(message, { ...options, outcomeUnknown: true }),
    );
  } catch (error) {
    // A rejected tee branch must also cancel the original; never wait for a
    // cancellation promise whose completion depends on that other branch.
    cancelUnreadResponseBody(response.body);
    throw error;
  }
  const code = platformErrorCode(
    await buffered.text(),
    response.headers.get("content-type"),
  );
  platformResponseErrors.set(response, code);
  return code;
}

export function authFailureMessage(status: number, authError: string | null) {
  if (status === 401) return AUTH_REQUIRED_MESSAGE;
  if (isCsrfAuthFailure(status, authError)) return PAGE_EXPIRED_MESSAGE;
  return "";
}

export async function assertResponse(
  response: Response,
  onUnauthorized?: (() => void) | undefined,
): Promise<Response> {
  const operationId = response.headers.get(OPERATION_ID_HEADER) || "";
  const operationState = response.headers.get(OPERATION_STATE_HEADER) || "";
  const authMessage = authFailureMessage(
    response.status,
    await responsePlatformErrorCode(response),
  );
  if (authMessage) {
    onUnauthorized?.();
    throw new AuthenticationError(authMessage, {
      status: response.status,
      code:
        response.status === 401
          ? "auth.session_required"
          : "auth.csrf_rejected",
      kind: response.status === 401 ? "authentication" : "csrf",
      operationId,
      operationState,
    });
  }
  const operationResult = classifyOperationResponse(
    response.status,
    operationState,
  );
  if (operationResult.kind === "success") return response;
  if (operationResult.kind === "invalid") {
    throw new RequestError(
      t("操作结果无效。{0}", "Invalid operation result. {0}", [
        RESULT_UNKNOWN_MESSAGE,
      ]),
      {
        status: response.status,
        code: "invalid_operation_result",
        kind: "protocol",
        outcomeUnknown: true,
        operationId,
        operationState: "unknown",
      },
    );
  }
  const detail = parseErrorPayload(
    await requestTextError(response),
    response.headers.get("content-type") || "",
  );
  const outcomeUnknown = operationResult.outcomeUnknown;
  const errorMetadata = {
    ...requestErrorMetadata(detail, response.headers),
    ...authoritativeOperationMetadata(
      operationId,
      operationState,
      detail.operation,
    ),
  };
  if (detail.status && detail.status !== response.status) {
    throw new RequestError(
      t(
        "错误响应无效：问题状态与 HTTP 状态不一致",
        "Invalid error response: problem status does not match HTTP status",
      ),
      {
        ...errorMetadata,
        status: response.status,
        code: "invalid_problem_status",
        kind: "protocol",
        outcomeUnknown,
        operationId: operationId || detail.operation?.id || "",
        operationState: operationState || detail.operation?.state || "",
      },
    );
  }
  throw new RequestError(problemMessage(detail, response.status), {
    ...errorMetadata,
    status: response.status,
    code: detail.code,
    kind: response.status === 409 ? "conflict" : "http",
    outcomeUnknown,
    operationId: operationId || detail.operation?.id || "",
    operationState: operationState || detail.operation?.state || "",
  });
}

/** Accept only the terminal envelope emitted after a staged upload is
 * discarded. Cleanup is successful only when the response is bound to the
 * expected upload and confirms the complete staged length was discarded.
 */
export async function assertDiscardUploadResponse(
  response: Response,
  expectedUploadId: string,
  expectedLength: number,
  onUnauthorized?: (() => void) | undefined,
): Promise<Response> {
  const classification = classifyUploadResponse({
    phase: "discard",
    errorCode: await responsePlatformErrorCode(response),
    status: response.status,
    headers: response.headers,
    expectedUploadId,
    expectedLength,
  });
  if (["authentication", "csrf"].includes(classification.kind)) {
    return assertResponse(response, onUnauthorized);
  }
  if (
    classification.kind === "rejected" &&
    classification.protocol?.offset === expectedLength
  ) {
    return response;
  }
  throw new RequestError(
    t(
      "暂存上传清理结果无效。{0}",
      "Invalid staged-upload cleanup result. {0}",
      [RESULT_UNKNOWN_MESSAGE],
    ),
    {
      status: response.status,
      code: "invalid_discard_result",
      kind: "protocol",
      outcomeUnknown: true,
      operationId: expectedUploadId,
      operationState: "unknown",
      uploadId: expectedUploadId,
      uploadState: "unknown",
      uploadLength: expectedLength,
    },
  );
}

export async function assertFreshUploadResponse(
  response: Response,
  expectedUploadId: string,
  expectedLength: number,
  onUnauthorized?: (() => void) | undefined,
): Promise<Response> {
  const classification = classifyUploadResponse({
    phase: "fresh",
    errorCode: await responsePlatformErrorCode(response),
    status: response.status,
    headers: response.headers,
    expectedUploadId,
    expectedLength,
  });
  if (["authentication", "csrf"].includes(classification.kind)) {
    return assertResponse(response, onUnauthorized);
  }
  if (classification.kind === "committed") return response;
  if (classification.kind !== "invalid") {
    const detail = parseErrorPayload(
      await requestTextError(response),
      response.headers.get("content-type") || "",
    );
    const errorMetadata = {
      ...requestErrorMetadata(detail, response.headers),
      ...authoritativeUploadMetadata(classification.protocol, detail.upload),
    };
    if (detail.status && detail.status !== response.status) {
      throw new RequestError(
        t(
          "错误响应无效：问题状态与 HTTP 状态不一致",
          "Invalid error response: problem status does not match HTTP status",
        ),
        {
          ...errorMetadata,
          status: response.status,
          code: "invalid_problem_status",
          kind: "protocol",
          outcomeUnknown: classification.outcomeUnknown,
          operationId: expectedUploadId,
          operationState: classification.kind,
          uploadId: expectedUploadId,
          uploadState: classification.kind,
        },
      );
    }
    throw new RequestError(problemMessage(detail, response.status), {
      ...errorMetadata,
      status: response.status,
      code: detail.code,
      kind: response.status === 409 ? "conflict" : "http",
      outcomeUnknown: classification.outcomeUnknown,
      operationId: expectedUploadId,
      operationState: classification.kind,
      uploadId: expectedUploadId,
      uploadState: classification.kind,
    });
  }
  throw new RequestError(
    t("上传结果无效。{0}", "Invalid upload result. {0}", [
      RESULT_UNKNOWN_MESSAGE,
    ]),
    {
      status: response.status,
      code: "invalid_upload_result",
      kind: "protocol",
      outcomeUnknown: true,
      operationId: expectedUploadId,
      operationState: "unknown",
    },
  );
}

export function isAuthenticationError(error: unknown) {
  return error instanceof AuthenticationError;
}

export function isRequestErrorCode(error: unknown, code: string) {
  return error instanceof RequestError && error.code === code;
}

export function parseErrorPayload(
  rawBody: unknown,
  contentType: string = "",
): Readonly<ParsedErrorPayload> {
  const body = typeof rawBody === "string" ? rawBody.trim() : "";
  let code = "";
  let message = "";
  let type = "";
  let title = "";
  let status = 0;
  let detail = "";
  let recovery = "";
  let retryAfter = null;
  let operation = null;
  let upload = null;
  if (body && isProblemMediaType(contentType)) {
    try {
      const payload = JSON.parse(body);
      if (isRecord(payload)) {
        type = normalizeBoundedString(payload.type, ERROR_TYPE_LIMIT);
        title = normalizeErrorMessageValue(payload.title);
        status = normalizeHttpStatus(payload.status);
        detail = normalizeErrorMessageValue(payload.detail);
        code = normalizeErrorCode(payload.code);
        recovery = normalizeRecoveryAdvice(payload.recovery);
        retryAfter = normalizeUnsignedInteger(payload.retry_after);
        operation = parseOperationExtension(payload);
        upload = parseUploadExtension(payload);
        message = detail || title;
      }
    } catch {
      // A malformed optional problem body is ignored in favor of HTTP status.
    }
  }
  return Object.freeze({
    type,
    title,
    status,
    detail,
    code,
    message,
    recovery,
    retryAfter,
    operation,
    upload,
  });
}

function requestErrorMetadata(
  detail: Readonly<ParsedErrorPayload>,
  headers: Headers | undefined,
): RequestErrorOptions {
  const retryAfterHeader = headers?.get("retry-after");
  return {
    problemStatus: detail.status,
    type: detail.type,
    title: detail.title,
    detail: detail.detail,
    recovery: detail.recovery,
    // Retry-After is transport metadata and therefore authoritative when the
    // server also repeats the delta in the Problem Details extension.
    retryAfter:
      retryAfterHeader === null || retryAfterHeader === undefined
        ? detail.retryAfter
        : normalizeRetryAfterHeader(retryAfterHeader),
    operationHttpStatus: detail.operation?.httpStatus || 0,
    operation: detail.operation,
    uploadId: detail.upload?.id || "",
    uploadState: detail.upload?.state || "",
    uploadLength: detail.upload?.length ?? null,
    uploadOffset: detail.upload?.offset ?? null,
    upload: detail.upload,
    sourceRevision: normalizeRevisionHeader(
      headers?.get(SOURCE_REVISION_HEADER),
    ),
    targetRevision: normalizeRevisionHeader(
      headers?.get(TARGET_REVISION_HEADER),
    ),
  };
}

function normalizeRevisionHeader(value: string | null | undefined) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value) ? value : "";
}

function authoritativeUploadMetadata(
  protocol: BoundUploadProtocol | null,
  bodyUpload: Readonly<UploadMetadata> | null,
): RequestErrorOptions {
  if (!protocol) return {};
  const upload = Object.freeze({
    ...(bodyUpload || {}),
    id: protocol.uploadId,
    state: protocol.state,
    length: protocol.length,
    offset: protocol.offset,
  });
  return {
    uploadId: protocol.uploadId,
    uploadState: protocol.state,
    uploadLength: protocol.length,
    uploadOffset: protocol.offset,
    upload,
  };
}

function authoritativeOperationMetadata(
  operationId: string,
  operationState: string,
  bodyOperation: Readonly<OperationMetadata> | null,
): RequestErrorOptions {
  if (!operationId && !operationState) return {};
  const operation = Object.freeze({
    id: operationId || bodyOperation?.id || "",
    state: operationState || bodyOperation?.state || "",
    httpStatus: bodyOperation?.httpStatus || 0,
  });
  return {
    operationId: operation.id,
    operationState: operation.state,
    operationHttpStatus: operation.httpStatus || 0,
    operation,
  };
}

function parseOperationExtension(
  payload: Record<string, unknown>,
): Readonly<OperationMetadata> | null {
  const id = normalizeOperationId(payload.operation_id);
  const state = normalizeErrorState(payload.state);
  if (!id || !state) return null;
  return Object.freeze({
    id,
    state,
    httpStatus: normalizeHttpStatus(payload.http_status),
  });
}

function parseUploadExtension(
  payload: Record<string, unknown>,
): Readonly<UploadMetadata> | null {
  const id = normalizeOperationId(payload.upload_id);
  const state = normalizeErrorState(payload.upload_state);
  if (!id || !state) return null;
  return Object.freeze({
    id,
    state,
    length: normalizeUnsignedInteger(payload.upload_length),
    offset: normalizeUnsignedInteger(payload.upload_offset),
  });
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

function isJobState(value: unknown): value is JobState {
  return (
    typeof value === "string" &&
    ["running", "succeeded", "failed", "unknown"].includes(value)
  );
}

function normalizeErrorCode(value: unknown): string {
  return typeof value === "string" && ERROR_CODE_PATTERN.test(value)
    ? value
    : "";
}

function normalizeErrorState(value: unknown): string {
  return typeof value === "string" && ERROR_STATE_PATTERN.test(value)
    ? value
    : "";
}

function normalizeRecoveryAdvice(value: unknown): string {
  return typeof value === "string" && RECOVERY_VALUES.has(value) ? value : "";
}

function normalizeOperationId(value: unknown): string {
  return typeof value === "string" && OPERATION_ID_PATTERN.test(value)
    ? value
    : "";
}

function normalizeHttpStatus(value: unknown): number {
  return typeof value === "number" &&
    Number.isInteger(value) &&
    value >= 100 &&
    value <= 599
    ? value
    : 0;
}

function normalizeUnsignedInteger(value: unknown): number | null {
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

function normalizeRetryAfterHeader(value: unknown): number | null {
  if (typeof value !== "string" || !/^(0|[1-9][0-9]*)$/.test(value)) {
    return null;
  }
  const seconds = Number(value);
  return Number.isSafeInteger(seconds) ? seconds : null;
}

function normalizeErrorMessageValue(value: unknown): string {
  return typeof value === "string" ? normalizeErrorMessage(value) : "";
}

function normalizeBoundedString(value: unknown, limit: number): string {
  if (typeof value !== "string") return "";
  const normalized = value.trim();
  return normalized && normalized.length <= limit ? normalized : "";
}

/** Fetch and parse one bounded JSON response while the request deadline remains
 * active. The response is returned alongside the payload so callers can still
 * validate status and protocol headers.
 */
export async function requestJson(
  url: RequestInfo | URL,
  init: RequestInit = {},
  options: RequestOptions = {},
): Promise<Readonly<RequestJsonResult>> {
  return await performRequest(url, init, options, async (response) => {
    const buffered = await bufferResponse(response, init.method, {
      outcomeUnknown: Boolean(options.outcomeUnknown),
      operationId: options.resultId || options.operationId || "",
      limit: options.limit,
    });
    if (!buffered.ok) {
      return Object.freeze({ response: buffered, payload: null });
    }
    if (!isJsonMediaType(buffered.headers.get("content-type"))) {
      await buffered.body?.cancel();
      throw new RequestError(
        t(
          "服务器返回了非 JSON 响应",
          "The server returned a non-JSON response",
        ),
        {
          status: response.status,
          code: "invalid_json_content_type",
          kind: "protocol",
          outcomeUnknown: Boolean(options.outcomeUnknown),
          operationId: options.resultId || options.operationId || "",
          operationState: options.outcomeUnknown ? "unknown" : "",
        },
      );
    }
    let payload;
    try {
      // Consume the bounded replay stream directly. Cloning it would tee the
      // stream and retain a second copy of every chunk in an unread branch.
      payload = JSON.parse(await buffered.text());
    } catch {
      throw new RequestError(
        t("服务器返回了无效 JSON", "The server returned invalid JSON"),
        {
          status: response.status,
          code: "invalid_json_response",
          kind: "protocol",
          outcomeUnknown: Boolean(options.outcomeUnknown),
          operationId: options.resultId || options.operationId || "",
          operationState: options.outcomeUnknown ? "unknown" : "",
        },
      );
    }
    return Object.freeze({ response: buffered, payload });
  });
}

/** Fetch a response that is expected to have no success body.
 */
export async function requestNoContent(
  url: RequestInfo | URL,
  init: RequestInit = {},
  options: RequestOptions = {},
): Promise<Response> {
  return await performRequest(url, init, options, async (response) => {
    const buffered = await bufferResponse(response, init.method, {
      outcomeUnknown: Boolean(options.outcomeUnknown),
      operationId: options.resultId || options.operationId || "",
      limit: ERROR_RESPONSE_BODY_LIMIT,
    });
    if (
      buffered.ok &&
      ![204, 205].includes(buffered.status) &&
      (await buffered.arrayBuffer()).byteLength !== 0
    ) {
      throw new RequestError(
        t(
          "服务器返回了意外的响应内容",
          "The server returned an unexpected response body",
        ),
        {
          status: buffered.status,
          code: "unexpected_response_body",
          kind: "protocol",
          outcomeUnknown: Boolean(options.outcomeUnknown),
          operationId: options.resultId || options.operationId || "",
          operationState: options.outcomeUnknown ? "unknown" : "",
        },
      );
    }
    return buffered;
  });
}

function isJsonMediaType(rawContentType: unknown): boolean {
  const mediaType = String(rawContentType || "")
    .split(";", 1)[0]
    .trim()
    .toLowerCase();
  return (
    mediaType === "application/json" ||
    (mediaType.startsWith("application/") && mediaType.endsWith("+json"))
  );
}

function isProblemMediaType(rawContentType: unknown): boolean {
  return (
    String(rawContentType || "")
      .split(";", 1)[0]
      .trim()
      .toLowerCase() === "application/problem+json"
  );
}

/** Fetch headers only; callers cannot accidentally extend the deadline gap.
 */
export async function requestHead(
  url: RequestInfo | URL,
  init: RequestInit = {},
  options: RequestOptions = {},
): Promise<Response> {
  return await performRequest(
    url,
    { ...init, method: "HEAD" },
    options,
    async (response) => response,
  );
}

async function performRequest<T>(
  url: RequestInfo | URL,
  init: RequestInit,
  options: RequestOptions,
  consume: (response: Response) => Promise<T>,
): Promise<T> {
  const timeoutMs = options.timeoutMs ?? REQUEST_TIMEOUT_MS;
  if (!Number.isSafeInteger(timeoutMs) || timeoutMs <= 0) {
    throw new TypeError(
      t(
        "请求超时时间必须为正整数",
        "Request timeout must be a positive integer",
      ),
    );
  }

  const callerSignal = init.signal;
  const outcomeUnknown = Boolean(options.outcomeUnknown);
  const operationId = options.operationId || "";
  const resultId = options.resultId || operationId;
  // No request can have reached the server when the caller supplies a signal
  // that was already aborted. Keep this distinct from cancellation after the
  // fetch dispatch, whose operation outcome must remain conservative.
  if (callerSignal?.aborted) {
    throw new RequestError(t("请求已取消。", "Request cancelled."), {
      code: "client_cancelled",
      kind: "cancelled",
      outcomeUnknown: false,
      operationId: resultId,
      operationState: "",
    });
  }

  const controller = new AbortController();
  let timedOut = false;
  const forwardAbort = () => controller.abort();
  callerSignal?.addEventListener("abort", forwardAbort, { once: true });
  const timer = window.setTimeout(() => {
    timedOut = true;
    controller.abort();
  }, timeoutMs);

  try {
    const response = await fetch(url, {
      ...init,
      signal: controller.signal,
    });
    const authMessage = authFailureMessage(
      response.status,
      await responsePlatformErrorCode(response),
    );
    // Authentication happens before the operation registry on the server, so
    // these responses intentionally do not carry an operation envelope. Let
    // assertResponse classify them and invoke the caller's auth callback.
    if (operationId && !authMessage) {
      const returnedOperationId =
        response.headers.get(OPERATION_ID_HEADER) || "";
      const returnedOperationState =
        response.headers.get(OPERATION_STATE_HEADER) || "";
      const validOperationStates = [
        "running",
        "succeeded",
        "failed",
        "rejected",
        "unknown",
      ];
      if (
        returnedOperationId !== operationId ||
        !validOperationStates.includes(returnedOperationState) ||
        (response.ok && returnedOperationState !== "succeeded") ||
        (!response.ok && returnedOperationState === "succeeded")
      ) {
        throw new RequestError(
          t("操作结果无效。{0}", "Invalid operation result. {0}", [
            RESULT_UNKNOWN_MESSAGE,
          ]),
          {
            status: response.status,
            code: "invalid_operation_result",
            kind: "protocol",
            outcomeUnknown: true,
            operationId,
            operationState: "unknown",
          },
        );
      }
    }
    return await consume(response);
  } catch (error) {
    if (error instanceof RequestError) throw error;
    if (timedOut) {
      throw new RequestError(
        options.timeoutMessage ||
          (outcomeUnknown
            ? t("请求超时。{0}", "Request timed out. {0}", [
                RESULT_UNKNOWN_MESSAGE,
              ])
            : t("请求超时，请重试。", "Request timed out. Try again.")),
        {
          code: "client_timeout",
          kind: "timeout",
          outcomeUnknown,
          operationId: resultId,
          operationState: outcomeUnknown ? "unknown" : "",
        },
      );
    }
    if (controller.signal.aborted) {
      throw new RequestError(
        outcomeUnknown
          ? t("请求已取消。{0}", "Request cancelled. {0}", [
              RESULT_UNKNOWN_MESSAGE,
            ])
          : t("请求已取消。", "Request cancelled."),
        {
          code: "client_cancelled",
          kind: "cancelled",
          outcomeUnknown,
          operationId: resultId,
          operationState: outcomeUnknown ? "unknown" : "",
        },
      );
    }
    throw new RequestError(
      outcomeUnknown
        ? t("网络连接中断。{0}", "Network connection lost. {0}", [
            RESULT_UNKNOWN_MESSAGE,
          ])
        : t("网络连接中断，请重试。", "Network connection lost. Try again."),
      {
        code: "network_error",
        kind: "network",
        outcomeUnknown,
        operationId: resultId,
        operationState: outcomeUnknown ? "unknown" : "",
      },
    );
  } finally {
    window.clearTimeout(timer);
    callerSignal?.removeEventListener("abort", forwardAbort);
  }
}

export function postJson(
  url: RequestInfo | URL,
  csrfToken: string,
  body: unknown,
  options: RequestOptions = {},
): Promise<Response> {
  const operationId = options.operationId || crypto.randomUUID();
  return requestNoContent(
    url,
    {
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        [CSRF_HEADER]: csrfToken,
        [OPERATION_ID_HEADER]: operationId,
      },
      body: JSON.stringify(body),
    },
    {
      ...options,
      outcomeUnknown: options.outcomeUnknown ?? true,
      operationId,
    },
  );
}

/** Fields shared by every successful job-status classification.
 */

/** Execute one mutation and reconcile an indeterminate response exactly once.
 */
export async function runMutationWithReconciliation(
  execute: () => Promise<Response>,
  onUnauthorized: (() => void) | undefined,
): Promise<MutationResult> {
  try {
    const response = await execute();
    await assertResponse(response, onUnauthorized);
    return Object.freeze({
      kind: "succeeded",
      response,
      status: null,
      reconciled: false,
    });
  } catch (error) {
    if (isAuthenticationError(error)) {
      return Object.freeze({ kind: "authentication" });
    }
    const jobId = jobIdForUnknownMutation(error);
    const status = jobId ? await queryJob(jobId, onUnauthorized) : null;
    if (status?.kind === "authentication") {
      return Object.freeze({ kind: "authentication" });
    }
    if (status?.kind === "succeeded") {
      return Object.freeze({
        kind: "succeeded",
        response: null,
        status,
        reconciled: true,
      });
    }
    return Object.freeze({ kind: "failed" as const, error, status });
  }
}

/** Query a job once and classify every possible result with `kind`.
 */
export async function queryJob(
  jobId: string,
  onUnauthorized?: (() => void) | undefined,
): Promise<JobQueryResult> {
  if (!OPERATION_ID_PATTERN.test(jobId)) {
    throw new TypeError(
      t("任务标识必须为规范的 UUID", "Job ID must be a canonical UUID"),
    );
  }

  try {
    const { response, payload } = await requestJson(
      `/__xczs__/api/jobs/${jobId}`,
      { method: "GET" },
      { outcomeUnknown: false },
    );
    if (!response.ok) await assertResponse(response, onUnauthorized);
    const responseOperationId = response.headers.get(OPERATION_ID_HEADER) || "";
    const responseOperationState =
      response.headers.get(OPERATION_STATE_HEADER) || "";
    if (!isRecord(payload)) {
      throw new RequestError(
        t("任务状态响应无效", "Invalid job status response"),
        {
          code: "invalid_job_status",
          kind: "protocol",
        },
      );
    }
    const payloadState = payload.state;
    const hasHttpStatus = payload.http_status !== undefined;
    const httpStatus = normalizeHttpStatus(payload.http_status);
    if (
      payload.job_id !== jobId ||
      responseOperationId !== jobId ||
      !isJobState(payloadState) ||
      responseOperationState !== payloadState ||
      (payloadState === "running" && hasHttpStatus) ||
      (payloadState === "succeeded" &&
        (httpStatus < 200 || httpStatus > 299)) ||
      (payloadState === "failed" && (httpStatus < 400 || httpStatus > 599)) ||
      (payloadState === "unknown" &&
        hasHttpStatus &&
        (httpStatus < 400 || httpStatus > 599))
    ) {
      throw new RequestError(
        t("任务状态响应无效", "Invalid job status response"),
        {
          code: "invalid_job_status",
          kind: "protocol",
        },
      );
    }
    return Object.freeze({
      kind: payloadState,
      jobId,
      state: payloadState,
      status: httpStatus,
      code: typeof payload.code === "string" ? payload.code : "",
      message: jobStatusMessage(payload),
      authenticationFailed: false,
    }) as JobQueryResult;
  } catch (statusError) {
    if (isAuthenticationError(statusError)) {
      return Object.freeze({
        kind: "authentication",
        jobId,
        state: "unknown",
        status: 0,
        code: "auth.session_required",
        message: AUTH_REQUIRED_MESSAGE,
        authenticationFailed: true,
      });
    }
    return Object.freeze({
      kind: "unavailable",
      jobId,
      state: "unknown",
      status: 0,
      code: "job_status_unavailable",
      message:
        `${RESULT_UNKNOWN_MESSAGE} ` +
        t("本次状态核对失败：{0}", "The one-time status check failed: {0}", [
          requestErrorMessage(statusError),
        ]),
      authenticationFailed: false,
    });
  }
}

function jobIdForUnknownMutation(error: unknown): string {
  return error instanceof RequestError &&
    error.outcomeUnknown &&
    OPERATION_ID_PATTERN.test(error.operationId)
    ? error.operationId
    : "";
}

export async function queryUnknownUpload(
  error: unknown,
  url: RequestInfo | URL,
  expectedLength: number,
  onUnauthorized?: (() => void) | undefined,
): Promise<Readonly<UploadQueryResult> | null> {
  if (
    !(error instanceof RequestError) ||
    !error.outcomeUnknown ||
    !OPERATION_ID_PATTERN.test(error.operationId)
  ) {
    return null;
  }

  try {
    const response = await requestHead(url, {
      headers: { [UPLOAD_ID_HEADER]: error.operationId },
    });
    const classification = classifyUploadResponse({
      phase: "checkpoint",
      errorCode: await responsePlatformErrorCode(response),
      status: response.status,
      headers: response.headers,
      expectedUploadId: error.operationId,
      expectedLength,
    });
    if (["authentication", "csrf"].includes(classification.kind)) {
      await assertResponse(response, onUnauthorized);
    }
    if (classification.kind === "invalid") {
      await assertResponse(response, onUnauthorized);
      throw new RequestError(
        t("上传状态响应无效", "Invalid upload status response"),
        {
          code: "invalid_upload_status",
          kind: "protocol",
        },
      );
    }
    return Object.freeze({
      operationId: error.operationId,
      state: classification.kind,
      status: response.status,
      code: "",
      message: uploadStatusMessage(classification.kind, response.status),
      authenticationFailed: false,
    });
  } catch (statusError) {
    if (isAuthenticationError(statusError)) {
      return Object.freeze({
        operationId: error.operationId,
        state: "unknown",
        status: 0,
        code: "auth.session_required",
        message: AUTH_REQUIRED_MESSAGE,
        authenticationFailed: true,
      });
    }
    return Object.freeze({
      operationId: error.operationId,
      state: "unknown",
      status: 0,
      code: "upload_status_unavailable",
      message:
        `${RESULT_UNKNOWN_MESSAGE} ` +
        t(
          "本次上传状态核对失败：{0}",
          "The one-time upload status check failed: {0}",
          [requestErrorMessage(statusError)],
        ),
      authenticationFailed: false,
    });
  }
}

export async function requestTextError(response: Response): Promise<string> {
  try {
    const buffered = await bufferResponse(response, "GET", {
      limit: ERROR_RESPONSE_BODY_LIMIT,
    });
    return await buffered.text();
  } catch {
    return "";
  }
}

function normalizeErrorMessage(value: string): string {
  const message = value.trim();
  if (!message || message.length > ERROR_MESSAGE_LIMIT) return "";
  return message;
}

function uploadStatusMessage(state: string, status: number): string {
  if (state === "committed") {
    return t(
      "服务器已确认上传提交成功。",
      "The server confirms that the upload was committed.",
    );
  }
  if (state === "rejected") {
    return t(
      "服务器已确认上传被拒绝。",
      "The server confirms that the upload was rejected.",
    );
  }
  if (state === "not-seen") {
    return t(
      "服务器没有记录此上传标识。",
      "The server did not record this upload ID.",
    );
  }
  if (state === "unknown") {
    if (status === 429) {
      return t(
        "状态查询繁忙，无法核对上传状态。请刷新文件夹后再重试。",
        "The upload status could not be checked because status queries are temporarily busy. Refresh the folder before trying again.",
      );
    }
    return status === 503
      ? t(
          "状态服务暂不可用，无法核对上传状态。请刷新文件夹后再重试。",
          "The upload status could not be checked because the status service is temporarily unavailable. Refresh the folder before trying again.",
        )
      : t(
          "服务器无法确认最终上传状态。请刷新文件夹后再重试。",
          "The server could not confirm the final upload status. Refresh the folder before trying again.",
        );
  }
  return (
    t(
      "服务器报告上传仍在进行。",
      "The server reports that the upload is still running. ",
    ) + t("请刷新文件夹后再重试。", "Refresh the folder before trying again.")
  );
}

function jobStatusMessage(payload: Record<string, unknown>): string {
  if (payload.state === "succeeded") {
    return t(
      "服务器已确认操作成功。",
      "The server confirms that the operation succeeded.",
    );
  }
  if (payload.state === "failed") {
    const detail = normalizeErrorMessageValue(payload.detail);
    return detail && getLocale() === "en"
      ? t(
          "服务器已确认操作失败：{0}",
          "The server confirms that the operation failed: {0}",
          [detail],
        )
      : t(
          "服务器已确认操作失败。",
          "The server confirms that the operation failed.",
        );
  }
  if (payload.state === "unknown") {
    return (
      t(
        "服务器无法证明最终操作结果。",
        "The server could not prove the final operation outcome. ",
      ) +
      t(
        "请刷新文件夹核对目标后再重试。",
        "Refresh the folder to inspect the target before trying again.",
      )
    );
  }
  return (
    t(
      "服务器报告操作仍在进行。",
      "The server reports that the operation is still running. ",
    ) + t("请刷新文件夹后再重试。", "Refresh the folder before trying again.")
  );
}

/** Protocol metadata stays intact; server prose is not a localized UI contract.
 */
function problemMessage(detail: Readonly<ParsedErrorPayload>, status: number) {
  if (getLocale() === "en" && detail.message) return detail.message;
  const labels = {
    path_exists: t("路径已存在", "Path already exists"),
    destination_exists: t("目标已存在", "Destination already exists"),
    source_changed: t(
      "源文件已变化，请刷新后核对",
      "Source changed; refresh and review",
    ),
    destination_changed: t(
      "目标已变化，请刷新后核对",
      "Destination changed; refresh and review",
    ),
    permission_denied: t("权限不足", "Permission denied"),
    not_found: t("目标不存在", "Target not found"),
    insufficient_storage: t("存储空间不足", "Insufficient storage"),
  } as Record<string, string>;
  return (
    labels[detail.code] ||
    t(
      "请求失败（HTTP {0}），请核对目标状态后再重试。",
      "Request failed (HTTP {0}); check the target state before retrying.",
      [status],
    )
  );
}

function requestErrorMessage(error: unknown): string {
  if (error instanceof Error && error.message) return error.message;
  return t("状态不可用", "status unavailable");
}
