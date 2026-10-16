export type UploadProtocolState =
  | "running"
  | "awaiting-confirmation"
  | "committed"
  | "rejected"
  | "not-seen"
  | "not-started"
  | "unknown";

export type UploadRequestPhase = "fresh" | "resume" | "checkpoint" | "discard";

export type FieldPresence = "absent" | "optional" | "required";

export type UploadStateRule = { length: FieldPresence; offset: FieldPresence };

export type BoundUploadProtocol = {
  uploadId: string;
  state: UploadProtocolState;
  length: number | null;
  offset: number | null;
};

export type UploadClassification = {
  kind: "authentication" | "csrf" | "invalid" | UploadProtocolState;
  protocol: BoundUploadProtocol | null;
  outcomeUnknown: boolean;
};

import { parseUnsignedHeader } from "../http/headers.ts";

export { parseUnsignedHeader } from "../http/headers.ts";

export const OPERATION_STATE_HEADER = "X-Xczs-Operation-State";
export const UPLOAD_ID_HEADER = "X-Xczs-Upload-Id";
export const UPLOAD_LENGTH_HEADER = "X-Xczs-Upload-Length";
export const UPLOAD_OFFSET_HEADER = "X-Xczs-Upload-Offset";
export const UPLOAD_OVERWRITE_HEADER = "X-Xczs-Upload-Overwrite";
export const TARGET_REVISION_HEADER = "X-Xczs-Target-Revision";
export const TARGET_REPLACEABLE_HEADER = "X-Xczs-Target-Replaceable";

export const FRESH_UPLOAD_SUCCESS_STATUSES = Object.freeze([200, 201]);
export const RESUME_UPLOAD_SUCCESS_STATUSES = Object.freeze([200, 204]);
export const FRESH_UPLOAD_ERROR_STATUSES = Object.freeze({
  running: Object.freeze([408, 409]),
  "awaiting-confirmation": Object.freeze([409]),
  rejected: Object.freeze([408, 409, 413, 500, 507]),
  "not-seen": Object.freeze([404]),
  "not-started": Object.freeze([403, 404, 408, 409, 429, 503]),
  unknown: Object.freeze([408, 500, 503, 504]),
});
export const RESUME_UPLOAD_ERROR_STATUSES = Object.freeze({
  running: Object.freeze([408, 409, 413, 500, 507]),
  "awaiting-confirmation": Object.freeze([408, 409, 413, 500, 507]),
  rejected: Object.freeze([408, 409, 413, 500, 507]),
  "not-seen": Object.freeze([404]),
  "not-started": Object.freeze([403, 404, 408, 409, 429, 503]),
  unknown: Object.freeze([408, 500, 503, 504]),
});
const CHECKPOINT_UPLOAD_STATUSES = Object.freeze({
  running: Object.freeze([200]),
  "awaiting-confirmation": Object.freeze([409]),
  committed: Object.freeze([200]),
  rejected: Object.freeze([409]),
  "not-seen": Object.freeze([404]),
  "not-started": Object.freeze([]),
  unknown: Object.freeze([429, 500, 503]),
});
const DISCARD_UPLOAD_STATUSES = Object.freeze({
  running: Object.freeze([]),
  "awaiting-confirmation": Object.freeze([]),
  committed: Object.freeze([]),
  rejected: Object.freeze([204]),
  "not-seen": Object.freeze([]),
  "not-started": Object.freeze([]),
  unknown: Object.freeze([]),
});

export const UPLOAD_RESPONSE_STATUS_MATRIX: Readonly<
  Record<
    UploadRequestPhase,
    Readonly<Record<UploadProtocolState, readonly number[]>>
  >
> = Object.freeze({
  fresh: Object.freeze({
    ...FRESH_UPLOAD_ERROR_STATUSES,
    committed: FRESH_UPLOAD_SUCCESS_STATUSES,
  }),
  resume: Object.freeze({
    ...RESUME_UPLOAD_ERROR_STATUSES,
    committed: RESUME_UPLOAD_SUCCESS_STATUSES,
  }),
  checkpoint: CHECKPOINT_UPLOAD_STATUSES,
  discard: DISCARD_UPLOAD_STATUSES,
});

const ABSENT = "absent";
const OPTIONAL = "optional";
const REQUIRED = "required";

const UPLOAD_STATE_RULES: Readonly<
  Record<UploadProtocolState, Readonly<UploadStateRule>>
> = Object.freeze({
  running: Object.freeze({
    length: REQUIRED,
    offset: REQUIRED,
  }),
  "awaiting-confirmation": Object.freeze({
    length: REQUIRED,
    offset: REQUIRED,
  }),
  committed: Object.freeze({
    length: REQUIRED,
    offset: REQUIRED,
  }),
  rejected: Object.freeze({
    length: REQUIRED,
    offset: OPTIONAL,
  }),
  "not-seen": Object.freeze({
    length: ABSENT,
    offset: ABSENT,
  }),
  "not-started": Object.freeze({
    length: REQUIRED,
    offset: OPTIONAL,
  }),
  unknown: Object.freeze({
    length: OPTIONAL,
    offset: OPTIONAL,
  }),
});

export const UPLOAD_PROTOCOL_STATES: readonly UploadProtocolState[] =
  Object.freeze(Object.keys(UPLOAD_STATE_RULES) as UploadProtocolState[]);

/** Classify one upload response using the complete HTTP-status/header matrix.
 * This function is deliberately side-effect free so XHR, fetch-based empty
 * uploads and checkpoint queries cannot drift into different interpretations.
 */
export function classifyUploadResponse(options: {
  phase: UploadRequestPhase;
  errorCode?: string | null;
  status: number;
  headers: Headers | ((name: string) => string | null);
  expectedUploadId: string;
  expectedLength: number;
}) {
  const { phase, status, headers, expectedUploadId, expectedLength } = options;
  if (
    !Number.isSafeInteger(status) ||
    status < 100 ||
    status > 599 ||
    !Object.hasOwn(UPLOAD_RESPONSE_STATUS_MATRIX, phase)
  ) {
    return uploadClassification("invalid", null, true);
  }

  if (status === 401) {
    return uploadClassification("authentication", null, false);
  }
  if (status === 403 && options.errorCode === "auth.csrf_rejected") {
    return uploadClassification("csrf", null, false);
  }

  const protocol = parseBoundUploadProtocol(
    headers,
    expectedUploadId,
    expectedLength,
  );
  if (!protocol) return uploadClassification("invalid", null, true);

  const acceptedStatuses = UPLOAD_RESPONSE_STATUS_MATRIX[phase][protocol.state];
  if (
    !acceptedStatuses.includes(status) ||
    (phase === "checkpoint" &&
      status === 429 &&
      protocol.state === "unknown" &&
      (protocol.length !== null || protocol.offset !== null)) ||
    (protocol.state === "committed" && protocol.offset !== expectedLength) ||
    (phase === "discard" &&
      protocol.state === "rejected" &&
      protocol.offset !== expectedLength)
  ) {
    return uploadClassification("invalid", protocol, true);
  }

  return uploadClassification(
    protocol.state,
    protocol,
    protocol.state === "unknown" ||
      (protocol.state === "running" && phase !== "checkpoint"),
  );
}

function uploadClassification(
  kind: UploadClassification["kind"],
  protocol: BoundUploadProtocol | null,
  outcomeUnknown: boolean,
): Readonly<UploadClassification> {
  return Object.freeze({ kind, protocol, outcomeUnknown });
}

/** Parse and validate an upload response against the selected file length.
 */
export function parseBoundUploadProtocol(
  headers: Headers | ((name: string) => string | null),
  expectedUploadId: string,
  expectedLength: number,
): BoundUploadProtocol | null {
  if (!Number.isSafeInteger(expectedLength) || expectedLength < 0) return null;
  const uploadId = readHeader(headers, UPLOAD_ID_HEADER) || "";
  const state = readHeader(headers, OPERATION_STATE_HEADER) || "";
  if (uploadId !== expectedUploadId || !isUploadProtocolState(state)) {
    return null;
  }
  const rule = UPLOAD_STATE_RULES[state];
  const rawLength = readHeader(headers, UPLOAD_LENGTH_HEADER);
  const rawOffset = readHeader(headers, UPLOAD_OFFSET_HEADER);
  const length = parseUnsignedHeader(rawLength);
  const offset = parseUnsignedHeader(rawOffset);
  if (
    !matchesBoundField(rawLength, length, rule.length, expectedLength, false)
  ) {
    return null;
  }
  if (
    !matchesBoundField(rawOffset, offset, rule.offset, expectedLength, true)
  ) {
    return null;
  }
  return { uploadId, state, length, offset };
}

/** Read a target revision only when it is a canonical 256-bit lowercase token.
 * The strict representation prevents a malformed conflict response from ever
 * becoming permission to overwrite a destination.
 */
export function parseTargetRevision(
  headers: Headers | ((name: string) => string | null),
): string | null {
  const revision = readHeader(headers, TARGET_REVISION_HEADER);
  return typeof revision === "string" && /^[0-9a-f]{64}$/.test(revision)
    ? revision
    : null;
}

export function parseTargetReplaceable(
  headers: Headers | ((name: string) => string | null),
): boolean | null {
  const replaceable = readHeader(headers, TARGET_REPLACEABLE_HEADER);
  if (replaceable === "true") return true;
  if (replaceable === "false") return false;
  return null;
}

function matchesBoundField(
  rawValue: string | null,
  parsedValue: number | null,
  presence: FieldPresence,
  expectedLength: number,
  isOffset: boolean,
) {
  if (presence === ABSENT) return rawValue === null;
  if (rawValue === null) return presence === OPTIONAL;
  if (parsedValue === null) return false;
  return isOffset
    ? parsedValue <= expectedLength
    : parsedValue === expectedLength;
}

function readHeader(
  headers: Headers | ((name: string) => string | null),
  name: string,
) {
  return typeof headers === "function" ? headers(name) : headers.get(name);
}

function isUploadProtocolState(value: string): value is UploadProtocolState {
  return Object.hasOwn(UPLOAD_STATE_RULES, value);
}
