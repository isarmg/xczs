export type TagRow = {
  id: number;
  name: string;
  color: string | null;
  file_count: number;
};

export type TagPage = {
  previous_cursor: string | null;
  next_cursor: string | null;
  tags: TagRow[];
};

export type TagLabel = Pick<TagRow, "id" | "name" | "color">;
export type ListedTags = {
  file_id: number | null;
  tags: TagLabel[];
  tags_has_more: boolean;
};
export const listedTags = (value: unknown): value is ListedTags =>
  object(value) &&
  (value.file_id === null || (integer(value.file_id) && value.file_id > 0)) &&
  Array.isArray(value.tags) &&
  value.tags.length <= PAGE_SIZE &&
  value.tags.every(
    (label) =>
      object(label) &&
      integer(label.id) &&
      label.id > 0 &&
      text(label.name, 320) &&
      (label.color === null ||
        (text(label.color, 7) && /^#[a-fA-F0-9]{6}$/.test(label.color))),
  ) &&
  typeof value.tags_has_more === "boolean";

export type FolderPage = {
  previous_cursor: string | null;
  next_cursor: string | null;
  folders: string[];
};

export const PAGE_SIZE = 50;
export const MAX_CURSOR_BYTES = 4096;

const object = (value: unknown): value is Record<string, unknown> =>
  value !== null && typeof value === "object" && !Array.isArray(value);

const text = (value: unknown, max: number): value is string =>
  typeof value === "string" &&
  new TextEncoder().encode(value).byteLength <= max;

const integer = (value: unknown): value is number =>
  typeof value === "number" && Number.isSafeInteger(value);

const finite = (value: unknown): value is number =>
  typeof value === "number" && Number.isFinite(value);

const cursor = (value: unknown) =>
  value === null ||
  (text(value, MAX_CURSOR_BYTES) && /^[A-Za-z0-9_-]+$/.test(value));

const page = (value: unknown) =>
  object(value) && cursor(value.previous_cursor) && cursor(value.next_cursor);

export const tagRow = (value: unknown): value is TagRow =>
  object(value) &&
  integer(value.id) &&
  value.id > 0 &&
  text(value.name, 320) &&
  (value.color === null ||
    (text(value.color, 7) && /^#[a-fA-F0-9]{6}$/.test(value.color))) &&
  integer(value.file_count) &&
  value.file_count >= 0;

export const tagsPage = (value: unknown): value is TagPage =>
  object(value) &&
  page(value) &&
  Array.isArray(value.tags) &&
  value.tags.length <= PAGE_SIZE &&
  value.tags.every(tagRow) &&
  new Set(value.tags.map((tag) => tag.id)).size === value.tags.length;

export const foldersPage = (value: unknown): value is FolderPage =>
  object(value) &&
  page(value) &&
  Array.isArray(value.folders) &&
  value.folders.length <= PAGE_SIZE &&
  value.folders.every(
    (name) => text(name, 768) && name.length > 0 && !name.includes("/"),
  ) &&
  new Set(value.folders).size === value.folders.length;

export const filesPage = (value: unknown): value is FilePage =>
  object(value) &&
  Array.isArray(value.files) &&
  value.files.length <= 100 &&
  integer(value.total) &&
  value.total >= 0 &&
  integer(value.page) &&
  value.page > 0 &&
  integer(value.page_size) &&
  value.page_size >= 1 &&
  value.page_size <= 100 &&
  value.files.length <= value.page_size &&
  value.files.every(
    (file) =>
      object(file) &&
      integer(file.id) &&
      file.id > 0 &&
      text(file.path, 4096) &&
      text(file.name, 768) &&
      typeof file.status === "string" &&
      ["present", "missing", "suspect"].includes(file.status) &&
      finite(file.size) &&
      file.size >= 0 &&
      finite(file.mtime_ns) &&
      Array.isArray(file.tag_ids) &&
      file.tag_ids.length <= PAGE_SIZE &&
      file.tag_ids.every((id) => integer(id) && id > 0) &&
      typeof file.tag_ids_has_more === "boolean",
  );

export const cursorQuery = (value: string | null) =>
  value === null ? "" : `?${new URLSearchParams({ cursor: value })}`;

export type CursorPage = Pick<TagPage, "previous_cursor" | "next_cursor">;
export type FileRow = {
  id: number;
  path: string;
  name: string;
  status: "present" | "missing" | "suspect";
  size: number;
  mtime_ns: number;
  tag_ids: number[];
  tag_ids_has_more: boolean;
};
export type FilePage = {
  files: FileRow[];
  total: number;
  page: number;
  page_size: number;
};
export type LibraryStatus = {
  root: string;
  available: boolean;
  last_scan_at: number | null;
  indexed: number;
  missing: number;
  suspect: number;
  last_error: string | null;
};
export const statusResponse = (value: unknown): value is LibraryStatus =>
  object(value) &&
  text(value.root, 4096) &&
  typeof value.available === "boolean" &&
  (value.last_scan_at === null || integer(value.last_scan_at)) &&
  integer(value.indexed) &&
  value.indexed >= 0 &&
  integer(value.missing) &&
  value.missing >= 0 &&
  integer(value.suspect) &&
  value.suspect >= 0 &&
  (value.last_error === null || typeof value.last_error === "string");
export const scanResponse = (value: unknown): value is { scanned: number } =>
  object(value) && integer(value.scanned) && value.scanned >= 0;
export const backupResponse = (value: unknown): value is { filename: string } =>
  object(value) && text(value.filename, 255) && value.filename.length > 0;
export const createdTagResponse = (value: unknown): value is { id: number } =>
  object(value) && integer(value.id) && value.id > 0;
