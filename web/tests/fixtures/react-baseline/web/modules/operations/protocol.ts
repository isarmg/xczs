export const OPERATION_ID_HEADER = "X-Xczs-Operation-Id";
export const OPERATION_STATE_HEADER = "X-Xczs-Operation-State";

const OPERATION_STATES = new Set([
  "running",
  "succeeded",
  "failed",
  "rejected",
  "unknown",
  "committed",
  "not-seen",
]);

export function classifyOperationResponse(
  status: number,
  state: string,
): Readonly<{
  kind: "success" | "error" | "invalid";
  outcomeUnknown: boolean;
}> {
  const ok = status >= 200 && status <= 299;
  if (ok && (!state || state === "succeeded")) {
    return Object.freeze({ kind: "success", outcomeUnknown: false });
  }
  if (
    (state && !OPERATION_STATES.has(state)) ||
    (ok && state !== "succeeded") ||
    (!ok && ["succeeded", "committed"].includes(state))
  ) {
    return Object.freeze({ kind: "invalid", outcomeUnknown: true });
  }
  return Object.freeze({
    kind: "error",
    outcomeUnknown: ["running", "unknown"].includes(state),
  });
}
