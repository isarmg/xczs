export type ResponseBufferOptions = {
  limit?: number;
  outcomeUnknown?: boolean;
  operationId?: string;
};

export type ResponseBufferErrorOptions = {
  status: number;
  code: string;
  kind: string;
  outcomeUnknown: boolean;
  operationId: string;
  operationState: string;
};

export type ResponseErrorFactory = (
  message: string,
  options: ResponseBufferErrorOptions,
) => Error;

import { t } from "../../platform.ts";
import { parseUnsignedHeader } from "./headers.ts";

export const ERROR_RESPONSE_BODY_LIMIT = 16 * 1024;
export const SUCCESS_RESPONSE_BODY_LIMIT = 16 * 1024 * 1024;

export async function bufferResponse(
  response: Response,
  method: string = "GET",
  options: ResponseBufferOptions = {},
  createError: ResponseErrorFactory = defaultErrorFactory,
): Promise<Response> {
  if (
    String(method).toUpperCase() === "HEAD" ||
    [204, 205, 304].includes(response.status)
  ) {
    return response;
  }
  const limit =
    options.limit ??
    (response.ok ? SUCCESS_RESPONSE_BODY_LIMIT : ERROR_RESPONSE_BODY_LIMIT);
  if (!Number.isSafeInteger(limit) || limit <= 0) {
    throw new TypeError(
      t(
        "响应大小限制必须为正整数",
        "Response body limit must be a positive integer",
      ),
    );
  }
  const declaredLength = parseUnsignedHeader(
    response.headers.get("content-length"),
  );
  if (declaredLength !== null && declaredLength > limit) {
    await cancelResponseBody(response.body);
    throw responseBodyTooLarge(response, options, createError);
  }
  if (!response.body) return response;

  const reader = response.body.getReader();

  const chunks: Uint8Array[] = [];
  let received = 0;
  try {
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      if (!(value instanceof Uint8Array)) {
        await cancelReader(reader);
        throw createError(t("响应数据流无效", "Invalid response body stream"), {
          status: response.status,
          code: "invalid_response_body",
          kind: "protocol",
          outcomeUnknown: Boolean(options.outcomeUnknown),
          operationId: options.operationId || "",
          operationState: options.outcomeUnknown ? "unknown" : "",
        });
      }
      if (value.byteLength > limit - received) {
        await cancelReader(reader);
        throw responseBodyTooLarge(response, options, createError);
      }
      chunks.push(value);
      received += value.byteLength;
    }
  } finally {
    reader.releaseLock();
  }

  return new Response(replayBufferedChunks(chunks), {
    status: response.status,
    statusText: response.statusText,
    headers: response.headers,
  });
}

function replayBufferedChunks(
  chunks: Uint8Array[],
): ReadableStream<Uint8Array> {
  let index = 0;
  return new ReadableStream({
    pull(controller) {
      if (index >= chunks.length) {
        chunks.length = 0;
        controller.close();
        return;
      }
      controller.enqueue(chunks[index++]);
    },
    cancel() {
      chunks.length = 0;
    },
  });
}

function responseBodyTooLarge(
  response: Response,
  options: ResponseBufferOptions,
  createError: ResponseErrorFactory,
) {
  return createError(
    t(
      "服务器响应超过允许的大小",
      "The server response exceeded the allowed size",
    ),
    {
      status: response.status,
      code: "response_body_too_large",
      kind: "protocol",
      outcomeUnknown: Boolean(options.outcomeUnknown),
      operationId: options.operationId || "",
      operationState: options.outcomeUnknown ? "unknown" : "",
    },
  );
}

function defaultErrorFactory(
  message: string,
  options: ResponseBufferErrorOptions,
) {
  return Object.assign(new Error(message), options);
}

function cancelResponseBody(body: ReadableStream<Uint8Array> | null) {
  try {
    void body?.cancel().catch(() => {});
  } catch {
    // Cancellation is best-effort after the response has already been rejected.
  }
}

function cancelReader(reader: ReadableStreamDefaultReader<Uint8Array>) {
  try {
    void reader.cancel().catch(() => {});
  } catch {
    // Cancellation is best-effort after the response has already been rejected.
  }
}
