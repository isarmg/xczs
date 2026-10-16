import { t } from "../../platform.ts";
export function currentPageUrl() {
  return location.href.split(/[?#]/)[0];
}

export function isValidLogicalPath(name: unknown): name is string {
  if (
    typeof name !== "string" ||
    name.length === 0 ||
    name.startsWith("/") ||
    name.includes("\0")
  ) {
    return false;
  }
  const encoder = new TextEncoder();
  return (
    encoder.encode(name).length <= 4095 &&
    name
      .split("/")
      .every(
        (part) =>
          part.length > 0 &&
          part !== "." &&
          part !== ".." &&
          encoder.encode(part).length <= 255,
      )
  );
}

export function isValidAbsoluteLogicalPath(path: unknown): path is string {
  return (
    typeof path === "string" &&
    path.startsWith("/") &&
    isValidLogicalPath(path.slice(1))
  );
}

export function childUrl(name: string, pageUrl: string = currentPageUrl()) {
  if (!isValidLogicalPath(name)) throw new Error(t("路径无效", "Invalid path"));
  const url = new URL(pageUrl);
  if (!url.pathname.endsWith("/")) url.pathname += "/";
  url.pathname += name.split("/").map(encodeURIComponent).join("/");
  return url.href;
}

export function logicalChildPath(basePath: string, name: string) {
  if (!isValidLogicalPath(name)) throw new Error(t("路径无效", "Invalid path"));
  const base = basePath.endsWith("/") ? basePath : `${basePath}/`;
  return `${base}${name}`;
}

export function browserUrlFromLogicalPath(path: string) {
  return location.origin + path.split("/").map(encodeURIComponent).join("/");
}
