export type UploadSelectionEntry = { file: File; name: string };

import { t } from "../../platform.ts";
import { isValidLogicalPath } from "../shared/path.ts";

export const UPLOAD_BATCH_FILE_LIMIT = 512;
export const UPLOAD_BATCH_PATH_BYTES_LIMIT = 256 * 1024;

/** Validate a selection before creating upload state or DOM.
 */
export function prepareUploadSelection(
  files: FileList | File[] | null | undefined,
  options: { fileLimit?: number; pathBytesLimit?: number } = {},
): {
  ok: boolean;
  error: string;
  entries: readonly UploadSelectionEntry[];
  totalPathBytes: number;
} {
  const fileLimit = options.fileLimit ?? UPLOAD_BATCH_FILE_LIMIT;
  const pathBytesLimit =
    options.pathBytesLimit ?? UPLOAD_BATCH_PATH_BYTES_LIMIT;
  if (!Number.isSafeInteger(fileLimit) || fileLimit <= 0) {
    throw new TypeError(
      t(
        "上传文件数限制必须为正整数",
        "Upload file limit must be a positive integer",
      ),
    );
  }
  if (!Number.isSafeInteger(pathBytesLimit) || pathBytesLimit <= 0) {
    throw new TypeError(
      t(
        "上传路径字节限制必须为正整数",
        "Upload path byte limit must be a positive integer",
      ),
    );
  }

  const entries: UploadSelectionEntry[] = [];
  const encoder = new TextEncoder();
  let totalPathBytes = 0;
  const length = files?.length || 0;
  for (let index = 0; index < length; index++) {
    if (entries.length >= fileLimit) {
      return Object.freeze({
        ok: false,
        error:
          t(
            "每批最多选择 {0} 个文件。",
            "Select no more than {0} files in one batch. ",
            [fileLimit],
          ) +
          t(
            "请将较大的文件夹分批选择。",
            "Split larger folders into multiple selections.",
          ),
        entries: Object.freeze([]),
        totalPathBytes,
      });
    }
    const file = files?.[index];
    if (!(file instanceof File)) {
      return Object.freeze({
        ok: false,
        error: t(
          "浏览器返回的文件选择无效。",
          "The browser returned an invalid file selection.",
        ),
        entries: Object.freeze([]),
        totalPathBytes,
      });
    }
    const name = file.webkitRelativePath || file.name;
    if (!isValidLogicalPath(name) || name.split("/").at(-1) !== file.name) {
      return Object.freeze({
        ok: false,
        error: t(
          "不支持所选路径 {0}。",
          "The selected path {0} is not supported.",
          [name || "(empty)"],
        ),
        entries: Object.freeze([]),
        totalPathBytes,
      });
    }
    totalPathBytes += encoder.encode(name).byteLength;
    if (totalPathBytes > pathBytesLimit) {
      return Object.freeze({
        ok: false,
        error:
          t(
            "所选路径超过每批 {0} 字节的限制。",
            "Selected paths exceed the {0}-byte batch limit. ",
            [pathBytesLimit],
          ) + t("请分批选择。", "Split the selection into smaller batches."),
        entries: Object.freeze([]),
        totalPathBytes,
      });
    }
    entries.push(Object.freeze({ file, name }));
  }
  return Object.freeze({
    ok: true,
    error: "",
    entries: Object.freeze(entries),
    totalPathBytes,
  });
}
