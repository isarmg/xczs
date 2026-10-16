export type IndexData = Readonly<{
  href: string;
  dir_exists: boolean;
  session: Readonly<import("@xcss/web/contracts").AdministratorSession>;
}>;

import { t } from "../../platform.ts";
import { isAdministratorSession } from "../../platform.ts";

/** Page metadata contains only business context. Session data must come from
 * the shared client's restore endpoint, never an HTML bootstrap field.
 */
export function parseIndexData(value: unknown, session: unknown): IndexData {
  if (!isPlainRecord(value))
    invalidIndexData(t("应为普通对象", "expected a plain object"));
  const keys = Reflect.ownKeys(value);
  if (
    keys.length !== 2 ||
    !keys.every((key) => key === "href" || key === "dir_exists")
  ) {
    invalidIndexData(
      t(
        "必须且只能包含 href 和 dir_exists",
        "expected exactly href and dir_exists",
      ),
    );
  }
  const href = ownDataValue(value, "href");
  const dirExists = ownDataValue(value, "dir_exists");
  if (!isCanonicalAbsoluteLogicalPath(href))
    invalidIndexData(
      t(
        "href 必须为规范的绝对逻辑路径",
        "href must be a canonical absolute logical path",
      ),
    );
  if (typeof dirExists !== "boolean")
    invalidIndexData(
      t("dir_exists 必须为布尔值", "dir_exists must be a boolean"),
    );
  if (!isAdministratorSession(session))
    invalidIndexData(
      t(
        "会话必须符合 xcss 合同",
        "session must satisfy the xcss contract",
      ),
    );
  return Object.freeze({
    href,
    dir_exists: dirExists,
    session: Object.freeze({ ...session }),
  });
}

function isPlainRecord(value: unknown): value is Record<string, unknown> {
  if (value === null || typeof value !== "object" || Array.isArray(value))
    return false;
  const prototype = Object.getPrototypeOf(value);
  return prototype === Object.prototype || prototype === null;
}

function ownDataValue(record: Record<string, unknown>, key: string): unknown {
  const descriptor = Object.getOwnPropertyDescriptor(record, key);
  if (!descriptor || !Object.hasOwn(descriptor, "value"))
    invalidIndexData(
      t("{0} 必须为数据属性", "{0} must be a data property", [key]),
    );
  return descriptor.value;
}

function isCanonicalAbsoluteLogicalPath(value: unknown): value is string {
  if (
    typeof value !== "string" ||
    !value.startsWith("/") ||
    value.includes("\0")
  )
    return false;
  return (
    value === "/" ||
    value
      .slice(1)
      .split("/")
      .every((part) => part.length > 0 && part !== "." && part !== "..")
  );
}

function invalidIndexData(reason: string): never {
  throw new TypeError(
    t("嵌入页面数据无效：{0}", "Invalid embedded index data: {0}", [reason]),
  );
}
