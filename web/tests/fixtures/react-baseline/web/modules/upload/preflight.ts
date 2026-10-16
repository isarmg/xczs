export type UploadPreflightTarget = {
  path: string;
  exists: boolean;
  revision: string | null;
  replaceable: boolean;
};

import { t } from "../../platform.ts";
import { isValidAbsoluteLogicalPath } from "../shared/path.ts";

/** Parse a preflight response and bind every result to the exact request path.
 * Order, cardinality and uniqueness are all checked so a partial or reordered
 * response cannot accidentally authorize replacement of a different target.
 */
export function parseUploadPreflight(
  payload: unknown,
  requestedPaths: readonly string[],
): readonly Readonly<UploadPreflightTarget>[] {
  if (
    !isRecord(payload) ||
    !Array.isArray(payload.targets) ||
    payload.targets.length !== requestedPaths.length ||
    new Set(requestedPaths).size !== requestedPaths.length ||
    !requestedPaths.every(isValidAbsoluteLogicalPath)
  ) {
    throw new TypeError(
      t("上传预检查响应无效", "Invalid upload preflight response"),
    );
  }

  return Object.freeze(
    payload.targets.map((value, index) => {
      const expectedPath = requestedPaths[index];
      if (
        !isRecord(value) ||
        value.path !== expectedPath ||
        typeof value.exists !== "boolean" ||
        typeof value.replaceable !== "boolean" ||
        !isRevisionOrNull(value.revision) ||
        (value.exists && value.revision === null) ||
        (!value.exists && value.revision !== null)
      ) {
        throw new TypeError(
          t("上传预检查响应无效", "Invalid upload preflight response"),
        );
      }
      return Object.freeze({
        path: expectedPath,
        exists: value.exists,
        revision: value.revision,
        replaceable: value.replaceable,
      });
    }),
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return Boolean(value) && typeof value === "object" && !Array.isArray(value);
}

function isRevisionOrNull(value: unknown): value is string | null {
  return (
    value === null ||
    (typeof value === "string" && /^[0-9a-f]{64}$/.test(value))
  );
}
