import { getLocale, t } from "@xcss/web/admin-ui/i18n";
import React, { useCallback, useEffect, useState } from "react";
import {
  Button,
  Checkbox,
  ConfirmDangerDialog,
  Dialog,
  EmptyState,
  ErrorState,
  FormField,
  LoadingState,
  Select,
  StatusBadge,
  Table,
  TextField,
} from "@xcss/web/admin-ui";
import "../tags.css";
import { ApiClientError } from "@xcss/web/http-client";
import { tagRequest } from "./tag-api.ts";
import type {
  TagRow,
  TagPage,
  FileRow,
  CursorPage,
  LibraryStatus,
} from "./tags-pages.ts";

type Filters = {
  search: string;
  scope: string;
  status: string;
  all: number[];
  any: number[];
  exclude: number[];
};
type DialogState =
  | { kind: "file-tags" | "relink"; file: FileRow }
  | { kind: "delete"; tag: TagRow }
  | { kind: "rename"; tag: TagRow; name: string; color: string };
import {
  filesPage,
  tagsPage,
  cursorQuery,
  statusResponse,
  scanResponse,
  backupResponse,
  createdTagResponse,
  foldersPage,
} from "./tags-pages.ts";

const empty = (value: unknown): value is undefined => value === undefined;
const defaults: Filters = {
  search: "",
  scope: "current",
  status: "present",
  all: [],
  any: [],
  exclude: [],
};
const selectedValues = (event: React.ChangeEvent<HTMLSelectElement>) =>
  Array.from(event.currentTarget.selectedOptions, (option) =>
    Number(option.value),
  );
function size(value: number) {
  if (value < 1024) return `${value} B`;
  const units = ["KiB", "MiB", "GiB", "TiB"];
  let index = -1;
  do {
    value /= 1024;
    index++;
  } while (value >= 1024 && index < units.length - 1);
  return `${value.toFixed(1)} ${units[index]}`;
}
const time = (ns: number) => new Date(ns / 1e6).toLocaleString(getLocale(), { timeZone: "UTC", timeZoneName: "short" });
const fileHref = (path: string) =>
  "/" + path.split("/").map(encodeURIComponent).join("/");
function failureMessage(cause: unknown) {
  const error = cause instanceof ApiClientError ? cause : null;
  if (error?.code === "tag_limit_exceeded")
    return t("每个文件最多添加 16 个标签，请先移除现有标签。", "Each file can have at most 16 tags. Remove an existing tag first.");
  if (error?.code === "capacity_exhausted")
    return t(
      "已达到持久容量限制。现有记录已保留，请先整理数据或备份文件。",
      "Storage capacity reached. Existing records are retained. Organize your data or backups before retrying.",
    );
  if (error?.status === 409)
    return t(
      "文件或标签状态已变化，请刷新后重试。",
      "File or tag state has changed. Refresh and try again.",
    );
  if (error?.status === 429)
    return t(
      "服务正忙，请稍后重试。",
      "The service is busy. Please try again later.",
    );
  if (error?.status === 503)
    return t(
      "文件目录暂时不可用。",
      "The file directory is temporarily unavailable.",
    );
  return t(
    "操作失败，请稍后重试。",
    "Operation failed. Please try again later.",
  );
}

function PageNavigation({
  label,
  page,
  onPage,
  disabled,
}: {
  label: string;
  page: CursorPage;
  onPage(cursor: string | null): void;
  disabled: boolean;
}) {
  return (
    <nav className="library-pager" aria-label={label}>
      <Button
        disabled={disabled || !page.previous_cursor}
        onClick={() => onPage(null)}
      >
        {t("首页", "First page")}
      </Button>
      <Button
        disabled={disabled || !page.previous_cursor}
        onClick={() => onPage(page.previous_cursor)}
      >
        {t("上一页", "Previous page")}
      </Button>
      <Button
        disabled={disabled || !page.next_cursor}
        onClick={() => onPage(page.next_cursor)}
      >
        {t("下一页", "Next page")}
      </Button>
    </nav>
  );
}

function IndexedFolders({ client, directory }: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
  directory: string;
}) {
  const [path, setPath] = useState(() => directory.replace(/\/+$/, ""));
  const [cursor, setCursor] = useState<string | null>(null);
  const [page, setPage] = useState<import("./tags-pages.ts").FolderPage | null>(null);
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  useEffect(() => {
    let live = true;
    const controller = new AbortController();
    setPage(null);
    setError("");
    const query = new URLSearchParams({ path });
    if (cursor) query.set("cursor", cursor);
    tagRequest(client, `/folders?${query}`, foldersPage, undefined, undefined, controller.signal)
      .then(value => { if (live) setPage(value); })
      .catch(cause => { if (live) setError(failureMessage(cause)); });
    return () => { live = false; controller.abort(); };
  }, [client, path, cursor, version]);
  function navigate(next: string) { setCursor(null); setPath(next); }
  return <section className="xcss-content-panel library-panel" aria-label={t("索引目录", "Indexed folders")}>
    <h2>{t("索引目录", "Indexed folders")}</h2>
    <p>{t("浏览已索引文件所在的目录。空目录不在索引中。", "Browse folders containing indexed files. Empty folders are not indexed.")}</p>
    <p><a href={fileHref(path) + (path ? "/" : "")}>{path || "/"}</a></p>
    {path && <Button onClick={() => navigate(path.split("/").slice(0, -1).join("/"))}>{t("上级目录", "Parent folder")}</Button>}
    {error ? <ErrorState onRetry={() => setVersion(value => value + 1)}>{error}</ErrorState> : !page ? <LoadingState /> : <>
      {page.folders.length ? <ul>{page.folders.map(name => {
        const child = path ? `${path}/${name}` : name;
        return <li key={name}><Button onClick={() => navigate(child)}>{name}</Button> <a href={fileHref(child) + "/"}>{t("打开文件目录", "Open folder")}</a></li>;
      })}</ul> : <EmptyState>{t("没有已索引的子目录。", "No indexed subfolders.")}</EmptyState>}
      <PageNavigation label={t("索引目录分页", "Indexed folder pages")} page={page} onPage={setCursor} disabled={false} />
    </>}
  </section>;
}
export function FileLibrary({
  client,
  view,
  directory,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
  view: "files" | "tags" | "status" | null;
  directory: string;
}) {
  const tagFilters = [
    ["all", t("全部标签", "All tags")],
    ["any", t("任一标签", "Any tag")],
    ["exclude", t("排除标签", "Exclude tags")],
  ] as const;
  const [notice, notify] = useState("");
  const [draft, setDraft] = useState(defaults);
  const [filters, setFilters] = useState(defaults);
  const [page, setPage] = useState(1);
  const [version, setVersion] = useState(0);
  const [files, setFiles] = useState<FileRow[]>([]);
  const [total, setTotal] = useState(0);
  const [tags, setTags] = useState<TagRow[]>([]);
  const [tagCursor, setTagCursor] = useState<string | null>(null);
  const [tagPage, setTagPage] = useState<CursorPage>({
    previous_cursor: null,
    next_cursor: null,
  });
  const [tagChoices, setTagChoices] = useState<TagRow[]>([]);
  const [fileTagCursor, setFileTagCursor] = useState<string | null>(null);
  const [fileTagPage, setFileTagPage] = useState<TagPage | null>(null);
  const [status, setStatus] = useState<LibraryStatus | null>(null);
  const [selected, setSelected] = useState<number[]>([]);
  const [batchTag, setBatchTag] = useState("");
  const [newTag, setNewTag] = useState("");
  const [newColor, setNewColor] = useState("#3b82f6");
  const [dialog, setDialog] = useState<DialogState | null>(null);
  const [pending, setPending] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const refresh = () => {
    setTagCursor(null);
    setFileTagCursor(null);
    setVersion((value) => value + 1);
  };
  const request = useCallback(
    <T,>(
      path: string,
      guard: (value: unknown) => value is T,
      method?: string,
      body?: unknown,
      signal?: AbortSignal,
    ): Promise<T> => tagRequest(client, path, guard, method, body, signal),
    [client],
  );
  async function perform(operation: () => Promise<unknown>, message: string, failure?: string) {
    setError("");
    setPending(true);
    try {
      await operation();
      window.dispatchEvent(new Event("xczs:tags-changed"));
      if (message) notify(message);
      refresh();
    } catch (cause) {
      setError(failure ?? failureMessage(cause));
      // An unconfirmed scan may have committed. Re-read status without replaying it.
      if (failure) refresh();
    } finally {
      setPending(false);
    }
  }

  useEffect(() => {
    notify("");
    setDialog(null);
  }, [view]);
  useEffect(() => {
    if (view !== "files" && view !== "tags") return;
    let live = true;
    const controller = new AbortController();
    request(
      `/tags${cursorQuery(tagCursor)}`,
      tagsPage,
      undefined,
      undefined,
      controller.signal,
    )
      .then((value) => {
        if (live) {
          setTags(value.tags);
          setTagPage(value);
        }
      })
      .catch((cause) => {
        if (live) setError(failureMessage(cause));
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [request, version, tagCursor, view]);
  useEffect(() => {
    const retained = new Set([
      ...draft.all,
      ...draft.any,
      ...draft.exclude,
      Number(batchTag),
    ]);
    setTagChoices((previous) => [
      ...new Map(
        [...previous.filter((tag) => retained.has(tag.id)), ...tags].map(
          (tag) => [tag.id, tag],
        ),
      ).values(),
    ]);
  }, [draft, batchTag, tags]);
  useEffect(() => {
    if (view !== "files") return;
    let live = true;
    const controller = new AbortController();
    const params = new URLSearchParams({
      search: filters.search,
      directory,
      scope: filters.scope,
      status: filters.status,
      page: String(page),
      page_size: "50",
    });
    for (const key of ["all", "any", "exclude"] as const)
      if (filters[key].length) params.set(key, filters[key].join(","));
    setLoading(true);
    (async () => {
      const result = await request(
        `/files?${params}`,
        filesPage,
        undefined,
        undefined,
        controller.signal,
      );
      if (!live) return;
      if (live) {
        setFiles(result.files);
        setTotal(result.total);
        setSelected([]);
        setLoading(false);
      }
    })().catch((cause) => {
      if (live) {
        setError(failureMessage(cause));
        setLoading(false);
      }
    });
    return () => {
      live = false;
      controller.abort();
    };
  }, [request, filters, directory, page, version, view]);
  useEffect(() => {
    if (dialog?.kind !== "file-tags") {
      setFileTagPage(null);
      return;
    }
    let live = true;
    const controller = new AbortController();
    setFileTagPage(null);
    request(
      `/files/${dialog.file.id}/tags${cursorQuery(fileTagCursor)}`,
      tagsPage,
      undefined,
      undefined,
      controller.signal,
    )
      .then((value) => {
        if (live) setFileTagPage(value);
      })
      .catch((cause) => {
        if (live) setError(failureMessage(cause));
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [request, dialog, fileTagCursor, version]);
  useEffect(() => {
    if (view !== "status") return;
    let live = true;
    const controller = new AbortController();
    request("/status", statusResponse, undefined, undefined, controller.signal)
      .then((value) => {
        if (live) setStatus(value);
      })
      .catch((cause) => {
        if (live) setError(failureMessage(cause));
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [request, view, version]);
  const tagName = (id: number) =>
    tagChoices.find((tag) => tag.id === id)?.name ?? `#${id}`;
  const updateFilter = <K extends keyof Filters>(key: K, value: Filters[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));
  const toggle = (id: number) =>
    setSelected((current) =>
      current.includes(id)
        ? current.filter((value) => value !== id)
        : [...current, id],
    );
  const selectable = files
    .filter((file) => file.status !== "missing")
    .map((file) => file.id);
  function batch(action: "add" | "remove") {
    const tagId = Number(batchTag);
    if (!selected.length || !tagId) {
      setError(t("请先选择文件和标签。", "Select files and a tag first."));
      return;
    }
    void perform(
      () =>
        request("/file-tags", empty, "POST", {
          file_ids: selected,
          tag_ids: [tagId],
          action,
        }),
      action === "add"
        ? t("标签已添加。", "Tag added.")
        : t("标签已移除。", "Tag removed."),
    );
  }

  return (
    <div className="library-page" hidden={view === null}>
      {notice && (
        <p className="library-notice" role="status">
          {notice}
        </p>
      )}
      {error && (
        <ErrorState
          onRetry={() => {
            setError("");
            refresh();
          }}
        >
          {error}
        </ErrorState>
      )}
      {view === "files" && (
        <>
          <form
            className="xcss-content-panel library-panel library-filters"
            onSubmit={(event) => {
              event.preventDefault();
              setPage(1);
              setFilters({ ...draft });
            }}
          >
            <FormField label={t("文件名", "File name")}>
              <TextField
                type="search"
                value={draft.search}
                onChange={(event) => updateFilter("search", event.target.value)}
                placeholder={t("搜索文件名", "Search file names")}
              />
            </FormField>
            <FormField label={t("范围", "Scope")}>
              <Select
                value={draft.scope}
                onChange={(event) => updateFilter("scope", event.target.value)}
              >
                <option value="current">
                  {t("当前目录", "Current directory")}
                </option>
                <option value="recursive">
                  {t("包含子目录", "Include subdirectories")}
                </option>
                <option value="all">{t("整个文件库", "Entire library")}</option>
              </Select>
            </FormField>
            <FormField label={t("索引状态", "Index status")}>
              <Select
                value={draft.status}
                onChange={(event) => updateFilter("status", event.target.value)}
              >
                <option value="present">
                  {t("当前文件", "Present files")}
                </option>
                <option value="suspect">{t("待确认", "Unconfirmed")}</option>
                <option value="missing">{t("缺失", "Missing")}</option>
                <option value="all">{t("全部记录", "All records")}</option>
              </Select>
            </FormField>
            {tagFilters.map(([key, label]) => (
              <FormField key={key} label={label}>
                <Select
                  multiple
                  aria-label={label}
                  size={Math.max(2, Math.min(4, tags.length))}
                  value={draft[key].map(String)}
                  onChange={(event) => {
                    const values = selectedValues(event);
                    if (values.length <= 100) updateFilter(key, values);
                    else
                      setError(
                        t(
                          "每个条件最多选择 100 个标签。",
                          "Select up to 100 tags per condition.",
                        ),
                      );
                  }}
                >
                  {tagChoices.map((tag) => (
                    <option key={tag.id} value={tag.id}>
                      {tag.name}
                    </option>
                  ))}
                </Select>
              </FormField>
            ))}
            <PageNavigation
              label={t("筛选标签分页", "Filter tag pages")}
              page={tagPage}
              onPage={setTagCursor}
              disabled={pending}
            />
            <div className="library-actions">
              <Button type="submit">{t("筛选", "Filter")}</Button>
              <Button
                onClick={() => {
                  setDraft(defaults);
                  setFilters(defaults);
                  setPage(1);
                }}
              >
                {t("清除筛选", "Clear filters")}
              </Button>
            </div>
          </form>
          <section
            className="xcss-content-panel library-panel"
            aria-label={t("文件列表", "File list")}
            aria-busy={loading}
          >
            <StatusBadge status={t("{0} 条记录", "{0} records", [total])} />
            <div className="library-batch">
              <span>
                {t("已选 {0} 项", "{0} items selected", [selected.length])}
              </span>
              <Select
                aria-label={t("批量操作的标签", "Tag for batch actions")}
                value={batchTag}
                onChange={(event) => setBatchTag(event.target.value)}
              >
                <option value="">{t("选择标签", "Select a tag")}</option>
                {tagChoices.map((tag) => (
                  <option key={tag.id} value={tag.id}>
                    {tag.name}
                  </option>
                ))}
              </Select>
              <Button
                disabled={pending || !selected.length}
                onClick={() => batch("add")}
              >
                {t("添加标签", "Add tag")}
              </Button>
              <Button
                disabled={pending || !selected.length}
                onClick={() => batch("remove")}
              >
                {t("移除标签", "Remove tag")}
              </Button>
            </div>
            {loading ? (
              <LoadingState />
            ) : files.length === 0 ? (
              <EmptyState>
                {t("没有符合条件的文件。", "No matching files.")}
              </EmptyState>
            ) : (
              <Table aria-label={t("文件列表", "File list")}>
                <thead>
                  <tr>
                    <th>
                      <Checkbox
                        aria-label={t(
                          "选择本页文件",
                          "Select files on this page",
                        )}
                        checked={
                          selectable.length > 0 &&
                          selectable.every((id) => selected.includes(id))
                        }
                        onChange={(event) =>
                          setSelected(event.target.checked ? selectable : [])
                        }
                      />
                    </th>
                    <th>{t("文件", "Files")}</th>
                    <th>{t("大小", "Size")}</th>
                    <th>{t("修改时间", "Modified at")}</th>
                    <th>{t("标签", "Tags")}</th>
                    <th>{t("状态", "Status")}</th>
                    <th>{t("操作", "Actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {files.map((file) => (
                    <tr key={file.id}>
                      <td>
                        <Checkbox
                          aria-label={t("选择 {0}", "Select {0}", [file.path])}
                          checked={selected.includes(file.id)}
                          disabled={file.status === "missing"}
                          onChange={() => toggle(file.id)}
                        />
                      </td>
                      <td className="library-file">
                        <strong>{file.name}</strong>
                        <small>
                          #{file.id} · {file.path}
                        </small>
                      </td>
                      <td>{size(file.size)}</td>
                      <td>{time(file.mtime_ns)}</td>
                      <td>
                        {file.tag_ids.map((id) => (
                          <span className="library-tag" key={id}>
                            {tagName(id)}
                          </span>
                        ))}
                        <Button
                          onClick={() => {
                            setFileTagCursor(null);
                            setDialog({ kind: "file-tags", file });
                          }}
                        >
                          {t("查看全部标签", "View all tags")}
                          {file.tag_ids_has_more
                            ? t("（分页）", "(paginated)")
                            : ""}
                        </Button>
                      </td>
                      <td>
                        <StatusBadge
                          status={
                            {
                              present: t("当前", "Present"),
                              suspect: t("待确认", "Unconfirmed"),
                              missing: t("缺失", "Missing"),
                            }[file.status] ?? file.status
                          }
                        />
                      </td>
                      <td>
                        <div className="library-row-actions">
                          {file.status !== "missing" && (
                            <>
                              <a href={fileHref(file.path)} download>
                                {t("下载", "Download")}
                              </a>
                              <a
                                href={`${fileHref(file.path.split("/").slice(0, -1).join("/"))}${file.path.includes("/") ? "/" : ""}#files`}
                              >
                                {t("打开所在目录", "Open folder")}
                              </a>
                              {file.status === "suspect" && (
                                <Button
                                  disabled={pending}
                                  onClick={() =>
                                    void perform(
                                      () =>
                                        request(
                                          `/files/${file.id}/confirm`,
                                          empty,
                                          "POST",
                                        ),
                                      t(
                                        "文件身份已确认。",
                                        "File identity confirmed.",
                                      ),
                                    )
                                  }
                                >
                                  {t("确认身份", "Confirm identity")}
                                </Button>
                              )}
                              <Button
                                disabled={pending}
                                onClick={() =>
                                  setDialog({ kind: "relink", file })
                                }
                              >
                                {t("重新关联标签", "Relink tags")}
                              </Button>
                            </>
                          )}
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )}
            <div className="library-pager">
              <Button disabled={page <= 1} onClick={() => setPage(page - 1)}>
                {t("上一页", "Previous page")}
              </Button>
              <span>
                {t("第 {0} / {1} 页", "Page {0} of {1}", [
                  page,
                  Math.max(1, Math.ceil(total / 50)),
                ])}
              </span>
              <Button
                disabled={page * 50 >= total}
                onClick={() => setPage(page + 1)}
              >
                {t("下一页", "Next page")}
              </Button>
            </div>
          </section>
        </>
      )}
      {view === "tags" && (
        <>
          <IndexedFolders client={client} directory={directory} />
          <form
            className="xcss-content-panel library-panel library-tag-form"
            onSubmit={(event) => {
              event.preventDefault();
              void perform(
                async () => {
                  await request("/tags", createdTagResponse, "POST", {
                    name: newTag,
                    color: newColor,
                  });
                  setNewTag((current) => current === newTag ? "" : current);
                },
                t("标签已创建。", "Tag created."),
              );
            }}
          >
            <FormField label={t("标签名称", "Tag name")}>
              <TextField
                value={newTag}
                onChange={(event) => setNewTag(event.target.value)}
                maxLength={80}
                required
              />
            </FormField>
            <FormField label={t("颜色", "Color")}>
              <TextField
                type="color"
                value={newColor}
                onChange={(event) => setNewColor(event.target.value)}
              />
            </FormField>
            <Button type="submit" disabled={pending}>
              {t("创建标签", "Create tag")}
            </Button>
          </form>
          <section className="xcss-content-panel library-panel">
            {tags.length === 0 ? (
              <EmptyState>
                {t("尚未创建标签。", "No tags created yet.")}
              </EmptyState>
            ) : (
              <Table aria-label={t("标签列表", "Tag list")}>
                <thead>
                  <tr>
                    <th>{t("标签", "Tags")}</th>
                    <th>{t("文件数", "File count")}</th>
                    <th>{t("操作", "Actions")}</th>
                  </tr>
                </thead>
                <tbody>
                  {tags.map((tag) => (
                    <tr key={tag.id}>
                      <td>
                        <span
                          className="library-color"
                          style={{ backgroundColor: tag.color || "#3b82f6" }}
                        />
                        {tag.name}
                      </td>
                      <td>{tag.file_count}</td>
                      <td>
                        <div className="library-row-actions">
                          <Button
                            onClick={() =>
                              setDialog({
                                kind: "rename",
                                tag,
                                name: tag.name,
                                color: tag.color || "#3b82f6",
                              })
                            }
                          >
                            {t("编辑", "Edit")}
                          </Button>
                          <Button
                            className="xcss-danger"
                            onClick={() => setDialog({ kind: "delete", tag })}
                          >
                            {t("删除整个标签", "Delete tag")}
                          </Button>
                        </div>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </Table>
            )}
            <PageNavigation
              label={t("标签列表分页", "Tag list pages")}
              page={tagPage}
              onPage={setTagCursor}
              disabled={pending}
            />
          </section>
        </>
      )}
      {view === "status" && (
        <>
          <section className="xcss-content-panel library-panel">
            <p>{t("重新扫描会更新整个文件库的索引。扫描期间请等待完成；若结果未能确认，请先检查扫描状态。", "Scanning updates the entire file library index. Wait for completion; if the result is unconfirmed, check scan status before retrying.")}</p>
            {status ? (
              <dl className="library-status">
                {[
                  [t("原文件根目录", "Original file root"), status.root],
                  [
                    t("目录状态", "Directory status"),
                    status.available
                      ? t("可用", "Available")
                      : t("异常", "Unavailable"),
                  ],
                  [
                    t("上次完整扫描", "Last full scan"),
                    status.last_scan_at
                      ? new Date(status.last_scan_at * 1000).toLocaleString(
                          getLocale(), { timeZone: "UTC", timeZoneName: "short" },
                        )
                      : t("尚未完成", "Not completed yet"),
                  ],
                  [t("当前文件", "Present files"), status.indexed],
                  [t("缺失记录", "Missing records"), status.missing],
                  [t("待确认记录", "Unconfirmed records"), status.suspect],
                  [
                    t("最近扫描错误", "Last scan error"),
                    status.last_error
                      ? t(
                          "扫描未能完成，请重新扫描。",
                          "The scan could not complete. Scan again.",
                        )
                      : t("无", "None"),
                  ],
                ].map(([label, value]) => (
                  <React.Fragment key={String(label)}>
                    <dt>{label}</dt>
                    <dd>{value}</dd>
                  </React.Fragment>
                ))}
              </dl>
            ) : (
              <LoadingState />
            )}
            <div className="library-actions">
              <Button disabled={pending} onClick={refresh}>{t("刷新扫描状态", "Refresh scan status")}</Button>
              <Button
                disabled={pending}
                onClick={() =>
                  void perform(async () => {
                    const value = await request("/scan", scanResponse, "POST");
                    notify(
                      t(
                        "扫描完成，读取 {0} 个文件。",
                        "Scan complete. Read {0} files.",
                        [value.scanned],
                      ),
                    );
                  }, "", t("扫描结果未能确认，请检查上次扫描时间和错误信息后再决定是否重试。", "The scan result could not be confirmed. Check the last scan time and errors before retrying."))
                }
              >
                {pending ? t("操作进行中…", "Operation in progress…") : t("重新扫描", "Scan again")}
              </Button>
              <Button
                disabled={pending}
                onClick={() =>
                  void perform(async () => {
                    const value = await request(
                      "/backup",
                      backupResponse,
                      "POST",
                    );
                    notify(
                      t("备份已保存：{0}", "Backup saved: {0}", [
                        value.filename,
                      ]),
                    );
                  }, "")
                }
              >
                {t("备份标签数据库", "Back up tag database")}
              </Button>
            </div>
          </section>
        </>
      )}
      {dialog?.kind === "file-tags" && (
        <Dialog
          title={t("文件标签：{0}", "File tags: {0}", [dialog.file.name])}
          onClose={() => setDialog(null)}
        >
          {fileTagPage ? (
            <>
              <ul>
                {fileTagPage.tags.map((tag) => (
                  <li key={tag.id}>{tag.name}</li>
                ))}
              </ul>
              <PageNavigation
                label={t("文件标签分页", "File tag pages")}
                page={fileTagPage}
                onPage={setFileTagCursor}
                disabled={pending}
              />
            </>
          ) : (
            <LoadingState />
          )}
        </Dialog>
      )}
      {dialog?.kind === "delete" && (
        <ConfirmDangerDialog
          title={t("删除整个标签", "Delete tag")}
          description={t(
            "删除“{0}”将移除它与 {1} 个文件的关联，原文件不会被删除。",
            "Deleting “{0}” removes its associations with {1} files. Original files are retained.",
            [dialog.tag.name, dialog.tag.file_count],
          )}
          pending={pending}
          onClose={() => setDialog(null)}
          onConfirm={() =>
            void perform(
              async () => {
                await request(`/tags/${dialog.tag.id}`, empty, "DELETE");
                setDialog((current) => current === dialog ? null : current);
              },
              t("标签已删除。", "Tag deleted."),
            )
          }
        >
          {t("请确认删除整个标签。", "Confirm deletion of this tag.")}
        </ConfirmDangerDialog>
      )}
      {dialog?.kind === "rename" && (
        <Dialog
          title={t("编辑标签", "Edit tag")}
          onClose={() => setDialog(null)}
        >
          <form
            className="library-dialog-form"
            onSubmit={(event) => {
              event.preventDefault();
              void perform(
                async () => {
                  await request(`/tags/${dialog.tag.id}`, empty, "PUT", {
                    name: dialog.name,
                    color: dialog.color,
                  });
                  setDialog((current) => current === dialog ? null : current);
                },
                t("标签已更新。", "Tag updated."),
              );
            }}
          >
            <FormField label={t("标签名称", "Tag name")}>
              <TextField
                value={dialog.name}
                maxLength={80}
                required
                onChange={(event) =>
                  setDialog({ ...dialog, name: event.target.value })
                }
              />
            </FormField>
            <FormField label={t("颜色", "Color")}>
              <TextField
                type="color"
                value={dialog.color}
                onChange={(event) =>
                  setDialog({ ...dialog, color: event.target.value })
                }
              />
            </FormField>
            <div className="library-actions">
              <Button onClick={() => setDialog(null)}>
                {t("取消", "Cancel")}
              </Button>
              <Button type="submit" disabled={pending}>
                {t("保存", "Save")}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
      {dialog?.kind === "relink" && (
        <Dialog
          title={t("重新关联标签", "Relink tags")}
          description={t(
            "将缺失文件记录的标签关联到“{0}”。只修改数据库。",
            "Relink tags from a missing file record to “{0}”. Only the database is updated.",
            [dialog.file.name],
          )}
          onClose={() => setDialog(null)}
        >
          <form
            className="library-dialog-form"
            onSubmit={(event) => {
              event.preventDefault();
              const sourceId = Number(
                new FormData(event.currentTarget).get("source_id"),
              );
              if (!Number.isSafeInteger(sourceId) || sourceId < 1) {
                setError(
                  t(
                    "请输入有效的缺失文件 ID。",
                    "Enter a valid missing file ID.",
                  ),
                );
                return;
              }
              void perform(
                async () => {
                  await request(
                    `/files/${dialog.file.id}/relink`,
                    empty,
                    "POST",
                    { source_id: sourceId },
                  );
                  setDialog(null);
                },
                t("标签已重新关联。", "Tags relinked."),
              );
            }}
          >
            <FormField label={t("缺失文件 ID", "Missing file ID")}>
              <TextField
                name="source_id"
                type="number"
                min="1"
                step="1"
                required
              />
            </FormField>
            <div className="library-actions">
              <Button onClick={() => setDialog(null)}>
                {t("取消", "Cancel")}
              </Button>
              <Button type="submit" disabled={pending}>
                {t("重新关联", "Relink")}
              </Button>
            </div>
          </form>
        </Dialog>
      )}
    </div>
  );
}
