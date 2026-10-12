export type MutationEffect =
  (typeof MUTATION_EFFECT)[keyof typeof MUTATION_EFFECT];

export type InlineRenameResult =
  "succeeded" | "retry" | "refresh" | "unknown" | "authentication";

export type IndexData = {
  href: string;
  dir_exists: boolean;
  session: {
    authenticated: true;
    user_id: string;
    username: string;
    role: "admin";
    csrf_token: string;
  };
};

export type ListingItem = {
  path_type: "Dir" | "SymlinkDir" | "File" | "SymlinkFile";
  name: string;
  mtime: number;
  size: number;
  revision: string;
  tags?: readonly import("../../react/tags-pages.ts").TagLabel[];
  fileId?: number | null;
  tagsHasMore?: boolean;
  tagsAvailable?: boolean;
};

export type InlineEditor = {
  index: number;
  sourceName: string;
  originalName: string;
  input: HTMLInputElement;
  error: HTMLElement;
  returnFocus: Element | null;
  created: boolean;
  blurFocus:
    | { control: "name" }
    | { element: Element }
    | null;
  commitPromise: Promise<InlineRenameResult> | null;
};

export type DirectoryListingOptions = {
  data: IndexData;
  params: {
    q: string;
    sort: string;
    order: string;
    all?: string;
    any?: string;
    exclude?: string;
  };
  table: HTMLTableElement;
  tableHead: HTMLTableSectionElement;
  tableBody: HTMLTableSectionElement;
  emptyFolder: HTMLElement;
  emptyNote: string;
  loadMore: HTMLButtonElement;
  listStatus: HTMLElement;
  onAction: (action: string, index: number) => void;
  onRename: (
    index: number,
    name: string,
    returnFocus: Element | null,
  ) => Promise<InlineRenameResult>;
  onUnauthorized: () => void;
  onTags: (file: ListingItem) => void;
};

import { t } from "../../platform.ts";
import {
  RequestError,
  assertResponse,
  isAuthenticationError,
  requestJson,
} from "../http/client.ts";
import {
  createElement,
  createIcon,
  errorMessage,
  formatFileSize,
} from "../shared/dom.ts";
import { MUTATION_EFFECT } from "../shared/mutation_effect.ts";
import { childUrl, isValidLogicalPath } from "../shared/path.ts";
import { listedTags } from "../../react/tags-pages.ts";
import { fileActionState, selectFileAction, subscribeFileAction, type FileAction } from "./action-mode.ts";

const LIST_PAGE_LIMIT = 200;
const MAX_CURSOR_LENGTH = 1024;
const MAX_RENDERED_ITEMS = LIST_PAGE_LIMIT;
const PATH_TYPES = new Set(["Dir", "SymlinkDir", "File", "SymlinkFile"]);

function isRefreshableDirectoryChange(error: unknown) {
  return (
    error instanceof RequestError &&
    error.status === 409 &&
    error.problemStatus === 409 &&
    error.code === "directory_changed" &&
    error.recovery === "refresh_target"
  );
}

export function createDirectoryListing(options: DirectoryListingOptions) {
  const {
    data,
    params,
    table,
    tableHead,
    tableBody,
    emptyFolder,
    emptyNote,
    loadMore,
    listStatus,
    onAction,
    onRename,
    onUnauthorized,
    onTags,
  } = options;

  let nextCursor: string | null = null;
  let loading = false;
  let loaded = false;
  let invalidated = false;
  let refreshAfterLoad = false;
  let revision = 0;
  let visibleCount = 0;
  let renderedStart = 0;
  let renderedEnd = 0;
  let renderedVisibleCount = 0;
  let invalidationMessage = "";
  let refreshAfterEditor = false;

  let activeEditor: InlineEditor | null = null;
  let selectedTagName: string | null = null;
  let choosing = false;

  const items: (ListingItem | null)[] = [];
  const loadedNames = new Set();
  const seenCursors = new Set();
  const showPrevious = createWindowButton(
    t("显示前面的项目", "Show previous items"),
  );
  const showNext = createWindowButton(t("显示后面的项目", "Show next items"));
  loadMore.before(showPrevious, showNext);

  function resetListing() {
    activeEditor = null;
    table.classList.remove("has-inline-editor");
    items.length = 0;
    visibleCount = 0;
    renderedStart = 0;
    renderedEnd = 0;
    renderedVisibleCount = 0;
    loadedNames.clear();
    seenCursors.clear();
    nextCursor = null;
    loaded = false;
    invalidated = false;
    invalidationMessage = "";
    refreshAfterEditor = false;
    tableBody.replaceChildren();
    table.classList.add("hidden");
    emptyFolder.classList.add("hidden");
    showPrevious.classList.add("hidden");
    showNext.classList.add("hidden");
  }

  function invalidate(effect: Exclude<MutationEffect, "not-committed">) {
    revision++;
    invalidated = true;
    nextCursor = null;
    seenCursors.clear();
    loadMore.disabled = loading;
    loadMore.textContent = t("刷新", "Refresh");
    loadMore.classList.remove("hidden");
    invalidationMessage =
      effect === MUTATION_EFFECT.OUTCOME_UNKNOWN
        ? t(
            "文件夹内容可能已变更，请刷新列表后再加载更多项目。",
            "Folder contents may have changed; refresh the list before loading more items.",
          )
        : effect === MUTATION_EFFECT.REFRESH_REQUIRED
          ? t(
              "文件夹快照已过时，请刷新列表后再操作。",
              "The folder snapshot is stale; refresh the list before another operation.",
            )
          : t(
              "文件夹内容已变更，请刷新列表后再加载更多项目。",
              "Folder contents changed; refresh the list before loading more items.",
            );
    listStatus.textContent = invalidationMessage;
  }

  /** Single cache/DOM invalidation boundary for every browser-side mutation.
   * Known rejections and pre-dispatch failures explicitly pass
   * `NOT_COMMITTED`, preventing conservative timeout handling from turning
   * every failed action into a needless refresh.
   */
  function notifyMutation(effect: MutationEffect): boolean {
    switch (effect) {
      case MUTATION_EFFECT.COMMITTED:
      case MUTATION_EFFECT.OUTCOME_UNKNOWN:
      case MUTATION_EFFECT.REFRESH_REQUIRED:
        invalidate(effect);
        return true;
      case MUTATION_EFFECT.NOT_COMMITTED:
        return false;
      default:
        throw new TypeError(t("变更结果无效", "Invalid mutation effect"));
    }
  }

  async function refreshFromFirstPage() {
    if (activeEditor) {
      refreshAfterEditor = true;
      return;
    }
    const focusAnchor = captureListingFocus();
    // Search and browser history mutate params without recreating this controller.
    renderHead();
    revision++;
    if (loading) {
      refreshAfterLoad = true;
      return;
    }
    resetListing();
    await loadNextPage();
    restoreListingFocus(focusAnchor);
  }

  /** Preserve the logical control, rather than a DOM node that the refresh will
   * remove. This keeps keyboard focus on the same item even when its row index
   * changes after a mutation.
   */
  function captureListingFocus():
    | {
        name: string;
        control: "name";
      }
    | "status"
    | null {
    const focused = document.activeElement;
    if (!(focused instanceof HTMLElement)) return null;
    if (
      focused === loadMore ||
      focused === showPrevious ||
      focused === showNext
    ) {
      return "status";
    }
    const row = focused.closest("tr[id^='addPath']");
    if (!(row instanceof HTMLTableRowElement) || !tableBody.contains(row)) {
      return null;
    }
    const rawIndex = row.id.slice("addPath".length);
    const index = Number(rawIndex);
    const item = Number.isSafeInteger(index) ? items[index] : null;
    if (!item) return "status";
    return { name: item.name, control: "name" };
  }

  function restoreListingFocus(
    anchor:
      | {
          name: string;
          control: "name";
        }
      | "status"
      | null,
  ) {
    // Do not steal focus if the user moved to another control while the
    // refresh request was in flight. A removed row leaves focus on body.
    if (!anchor || document.activeElement !== document.body) return;
    if (anchor === "status") {
      focusListStatus();
      return;
    }
    const index = items.findIndex((item) => item?.name === anchor.name);
    const row = index < 0 ? null : document.getElementById(`addPath${index}`);
    const selector = ".cell-name a";
    const target = row?.querySelector(selector);
    if (target instanceof HTMLElement) {
      target.focus({ preventScroll: true });
    } else {
      focusListStatus();
    }
  }

  function focusListStatus() {
    listStatus.tabIndex = -1;
    listStatus.focus({ preventScroll: true });
  }

  function renderHead() {
    const headerItems = [
      { name: "name", colspan: 2, text: t("名称", "Name") },
      { name: "mtime", text: t("修改时间", "Modified") },
      { name: "size", text: t("大小", "Size") },
    ];
    const row = createElement("tr");
    for (const item of headerItems) {
      let order = "desc";
      let indicator = "↕";
      const active = params.sort === item.name;
      if (active) {
        if (params.order === "desc") {
          order = "asc";
          indicator = "↓";
        } else {
          indicator = "↑";
        }
      }
      const query = new URLSearchParams({
        ...Object.fromEntries(
          Object.entries(params).filter(
            ([, value]) => typeof value === "string" && value !== "",
          ),
        ),
        order,
        sort: item.name,
      }).toString();
      const cell = createElement("th", {
        className: `cell-${item.name}`,
        attributes: {
          scope: "col",
          colspan: item.colspan,
          "aria-sort": active
            ? params.order === "desc"
              ? "descending"
              : "ascending"
            : "none",
        },
      });
      const link = createElement("a", {
        text: item.text,
        attributes: {
          href: `?${query}`,
          "aria-label": `${item.text}, ${active ? t("更改排序方向", "change sort direction") : t("按此列排序", "sort by this column")}`,
        },
      });
      link.append(
        createElement("span", {
          text: indicator,
          attributes: { "aria-hidden": "true" },
        }),
      );
      cell.append(link);
      row.append(cell);
      if (item.name === "name")
        row.append(
          createElement("th", {
            className: "cell-tags",
            text: t("标签", "Tags"),
            attributes: { scope: "col" },
          }),
        );
    }
    tableHead.replaceChildren(row);
  }

  async function loadNextPage() {
    if (invalidated) {
      await refreshFromFirstPage();
      return;
    }
    if (loading || (loaded && nextCursor === null)) return;
    const requestRevision = revision;
    const invokedWithFocus = document.activeElement === loadMore;
    loading = true;
    loadMore.textContent = t("加载更多", "Load more");
    loadMore.disabled = true;
    if (!loaded) loadMore.classList.add("hidden");
    listStatus.textContent = loaded
      ? t("正在加载更多…", "Loading more…")
      : t("正在加载文件…", "Loading files…");

    try {
      const url = new URL("/__xczs__/api/list", location.origin);
      url.searchParams.set("path", data.href);
      url.searchParams.set("limit", String(LIST_PAGE_LIMIT));
      if (params.q) url.searchParams.set("q", params.q);
      for (const key of ["all", "any", "exclude"] as const)
        if (params[key]) url.searchParams.set(key, params[key]);
      if (params.sort) url.searchParams.set("sort", params.sort);
      if (params.order) url.searchParams.set("order", params.order);
      if (nextCursor !== null) url.searchParams.set("cursor", nextCursor);

      const requestedCursor = nextCursor;
      let rawPayload;
      for (let attempt = 0; attempt < 2; attempt++) {
        const { response, payload } = await requestJson(url);
        if (requestRevision !== revision) return;
        if (response.status === 409 && loaded) {
          resetListing();
          throw new Error(
            t(
              "文件夹内容已变更，请重新加载列表后再重试。",
              "Folder contents changed. Reload the list and try again.",
            ),
          );
        }
        try {
          await assertResponse(response, onUnauthorized);
        } catch (error) {
          if (
            attempt === 0 &&
            requestedCursor === null &&
            isRefreshableDirectoryChange(error)
          ) {
            continue;
          }
          throw error;
        }
        rawPayload = payload;
        break;
      }
      const payload = validateListingPage(rawPayload);
      if (
        payload.nextCursor !== null &&
        (payload.nextCursor === requestedCursor ||
          seenCursors.has(payload.nextCursor))
      ) {
        throw new Error(
          t(
            "服务器重复返回了文件列表游标",
            "The server repeated a file list cursor",
          ),
        );
      }
      for (const file of payload.paths) {
        if (loadedNames.has(file.name)) {
          throw new Error(
            t(
              "服务器重复返回了文件列表项目",
              "The server repeated a file list item",
            ),
          );
        }
      }

      const firstIndex = items.length;
      const canAppendWithoutWindowing =
        items.length + payload.paths.length <= MAX_RENDERED_ITEMS;
      const fragment = canAppendWithoutWindowing
        ? document.createDocumentFragment()
        : null;
      if (fragment) {
        for (let offset = 0; offset < payload.paths.length; offset++) {
          addPath(payload.paths[offset], firstIndex + offset, fragment);
        }
      }

      items.push(...payload.paths);
      visibleCount += payload.paths.length;
      for (const file of payload.paths) loadedNames.add(file.name);
      nextCursor = payload.nextCursor;
      if (nextCursor !== null) seenCursors.add(nextCursor);
      loaded = true;
      if (fragment) {
        tableBody.append(fragment);
        renderedStart = 0;
        renderedEnd = items.length;
        renderedVisibleCount = visibleCount;
      } else if (!activeEditor) {
        // A rename can begin while this page is loading. Keep its row visible;
        // the new items remain available through the existing window controls.
        renderWindow(Math.max(0, items.length - MAX_RENDERED_ITEMS));
      }
      const moveFocusFromLoadMore =
        invokedWithFocus &&
        (document.activeElement === loadMore ||
          document.activeElement === document.body);
      updateVisibility(moveFocusFromLoadMore);
    } catch (error) {
      if (isAuthenticationError(error)) return;
      listStatus.textContent = t(
        "无法加载文件列表：{0}",
        "Unable to load the file list: {0}",
        [errorMessage(error)],
      );
      loadMore.textContent = t("重试", "Retry");
      loadMore.classList.remove("hidden");
      if (invokedWithFocus) loadMore.focus();
    } finally {
      loading = false;
      loadMore.disabled = false;
      if (refreshAfterLoad) {
        refreshAfterLoad = false;
        void refreshFromFirstPage();
      }
    }
  }

  function updateVisibility(moveFocusFromLoadMore = false) {
    if (visibleCount > 0) {
      table.classList.remove("hidden");
      emptyFolder.classList.add("hidden");
    } else {
      table.classList.add("hidden");
      emptyFolder.textContent =
        params.q || params.all || params.any || params.exclude
          ? t("没有搜索结果", "No search results")
          : emptyNote;
      emptyFolder.classList.remove("hidden");
    }

    if (invalidated) {
      loadMore.textContent = t("刷新", "Refresh");
      loadMore.classList.remove("hidden");
      listStatus.textContent = invalidationMessage;
    } else if (nextCursor === null) {
      if (moveFocusFromLoadMore) {
        focusListStatus();
      }
      loadMore.classList.add("hidden");
      listStatus.textContent =
        visibleCount > 0
          ? appendWindowStatus(
              t("已加载全部 {0} 项", "All {0} items loaded", [visibleCount]),
            )
          : "";
    } else {
      loadMore.classList.toggle("hidden", renderedEnd < items.length);
      listStatus.textContent = appendWindowStatus(
        t("已加载 {0} 项", "{0} items loaded", [visibleCount]),
      );
    }
    showPrevious.classList.toggle("hidden", renderedStart === 0);
    showNext.classList.toggle("hidden", renderedEnd >= items.length);
  }

  function appendWindowStatus(status: string) {
    if (renderedStart === 0 && renderedEnd >= items.length) return status;
    const first = items.length === 0 ? 0 : renderedStart + 1;
    return (
      t("{0}；显示第 {1}–{2} 项", "{0}; showing items {1}–{2} ", [
        status,
        first,
        renderedEnd,
      ]) +
      t("（当前窗口可见 {0} 项）", "({0} visible) in this window", [
        renderedVisibleCount,
      ])
    );
  }

  function renderWindow(start: number) {
    const maximumStart = Math.max(0, items.length - MAX_RENDERED_ITEMS);
    renderedStart = Math.max(0, Math.min(start, maximumStart));
    renderedEnd = Math.min(items.length, renderedStart + MAX_RENDERED_ITEMS);
    renderedVisibleCount = 0;
    const fragment = document.createDocumentFragment();
    for (let index = renderedStart; index < renderedEnd; index++) {
      const file = items[index];
      if (!file) continue;
      renderedVisibleCount++;
      addPath(file, index, fragment);
    }
    tableBody.replaceChildren(fragment);
  }

  function showPreviousWindow() {
    renderWindow(renderedStart - MAX_RENDERED_ITEMS);
    updateVisibility();
    if (renderedStart === 0) {
      focusListStatus();
    } else {
      showPrevious.focus();
    }
  }

  function showNextWindow() {
    renderWindow(renderedEnd);
    updateVisibility();
    const focusTarget =
      renderedEnd < items.length
        ? showNext
        : nextCursor === null
          ? listStatus
          : loadMore;
    if (focusTarget === listStatus) {
      focusListStatus();
    } else {
      focusTarget.focus();
    }
  }

  /** Add a known-committed item ahead of the current server snapshot. Rendering
   * stays inside the bounded row window, so the transient editor can never
   * create an extra DOM row. A stale row with the same name is replaced rather
   * than duplicated.
   */
  function addCreatedItem(file: ListingItem): number {
    const item = validateCreatedItem(file);
    const hadInlineEditor =
      activeEditor !== null ||
      tableBody.querySelector(
        ".inline-name-input, .inline-name-error, .is-renaming",
      ) !== null;
    activeEditor = null;
    table.classList.remove("has-inline-editor");
    const staleIndex = items.findIndex(
      (candidate) => candidate?.name === item.name,
    );
    const renderedRows = Array.from(tableBody.rows);
    const renderedRowIndices = renderedRows.map((row) =>
      Number(row.id.match(/^addPath(\d+)$/)?.[1] ?? Number.NaN),
    );
    const canPrepend =
      !hadInlineEditor &&
      staleIndex < 0 &&
      renderedStart === 0 &&
      new Set(renderedRowIndices).size === renderedRowIndices.length &&
      renderedRowIndices.every(
        (index, position) =>
          Number.isSafeInteger(index) &&
          index >= renderedStart &&
          index < renderedEnd &&
          pathRowUsesIndex(renderedRows[position], index),
      );
    if (staleIndex >= 0) {
      items.splice(staleIndex, 1);
      visibleCount = Math.max(0, visibleCount - 1);
      loadedNames.delete(item.name);
    }
    items.unshift(item);
    visibleCount++;
    loadedNames.add(item.name);
    loaded = true;
    invalidate(MUTATION_EFFECT.COMMITTED);
    if (canPrepend) {
      renderedEnd = Math.min(items.length, MAX_RENDERED_ITEMS);
      const indexedRows = renderedRows.map((row, position) => ({
        row,
        previousIndex: renderedRowIndices[position],
      }));
      indexedRows.sort(
        (left, right) => right.previousIndex - left.previousIndex,
      );
      for (const { row, previousIndex } of indexedRows) {
        const nextIndex = previousIndex + 1;
        if (nextIndex >= renderedEnd) {
          row.remove();
        } else {
          reindexPathRow(row, nextIndex);
        }
      }
      tableBody.prepend(createPathRow(item, 0));
      renderedVisibleCount = tableBody.rows.length;
    } else {
      renderWindow(0);
    }
    updateVisibility();
    return 0;
  }

  /** Keep one filename editor for the whole table. Starting another editor
   * first settles the current one, preventing two rows from racing a rename.
   */
  async function startInlineRename(
    index: number,
    returnFocus: Element | null = null,
    options: { created?: boolean } = {},
  ): Promise<boolean> {
    if (activeEditor?.index === index) {
      activeEditor.input.focus({ preventScroll: true });
      return true;
    }
    if (!(await settleInlineRename())) return false;
    const item = items[index];
    const row = document.getElementById(`addPath${index}`);
    const nameCell = row?.querySelector(".cell-name");
    if (
      !item ||
      !(row instanceof HTMLTableRowElement) ||
      !(nameCell instanceof HTMLTableCellElement) ||
      !tableBody.contains(row)
    ) {
      return false;
    }

    const errorId = `inlineNameError${index}`;
    const originalName = logicalBasename(item.name);
    const input = createElement("input", {
      className: "inline-name-input",
      attributes: {
        type: "text",
        autocomplete: "off",
        spellcheck: "false",
        "aria-label": t("重命名 {0}", "Rename {0}", [item.name]),
        "aria-describedby": errorId,
      },
    }) as HTMLInputElement;
    input.value = originalName;
    const error = createElement("span", {
      className: "inline-name-error",
      attributes: {
        id: errorId,
        role: "alert",
        hidden: true,
      },
    });
    const editor = {
      index,
      sourceName: item.name,
      originalName,
      input,
      error,
      returnFocus: returnFocus || document.querySelector('[data-file-action="rename"]'),
      created: Boolean(options.created),
      blurFocus: null,
      commitPromise: null,
    } as InlineEditor;
    activeEditor = editor;
    table.classList.add("has-inline-editor");
    row.classList.add("is-renaming");
    nameCell.replaceChildren(input, error);

    input.addEventListener("input", () => clearInlineError(editor));
    input.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && !event.isComposing) {
        event.preventDefault();
        void cancelInlineRename(editor, true);
      } else if (event.key === "Enter" && !event.isComposing) {
        event.preventDefault();
        void commitInlineRename(editor, true);
      }
    });
    input.addEventListener("blur", (event) => {
      if (editor.blurFocus === null) {
        editor.blurFocus = captureBlurFocus(index, event.relatedTarget);
      }
      void commitInlineRename(editor, false);
    });

    input.focus({ preventScroll: true });
    const isDirectory = item.path_type.endsWith("Dir");
    placeInlineCaret(input, isDirectory);
    return true;
  }

  async function settleInlineRename(): Promise<boolean> {
    const editor = activeEditor;
    if (!editor) return true;
    return await commitInlineRename(editor, false);
  }

  async function commitInlineRename(
    editor: InlineEditor,
    keepInvalid: boolean,
  ): Promise<boolean> {
    if (activeEditor !== editor) return true;
    if (editor.commitPromise) {
      await editor.commitPromise;
      return activeEditor !== editor;
    }

    const name = editor.input.value;
    if (!isValidInlineName(name)) {
      if (keepInvalid) {
        showInlineError(
          editor,
          t(
            "名称不能为空，不能包含斜杠或空字符，最多 255 个 UTF-8 字节。",
            "Use one non-empty name without '/' or NUL, at most 255 UTF-8 bytes.",
          ),
        );
        editor.input.focus({ preventScroll: true });
        return false;
      }
      await cancelInlineRename(editor, false);
      return true;
    }
    if (name === editor.originalName) {
      await cancelInlineRename(editor, keepInvalid);
      return true;
    }

    const shouldRestoreNameFocus = document.activeElement === editor.input;
    // Install the single in-flight guard before disabling the focused input.
    // Chromium synchronously fires blur when a focused control is disabled;
    // the re-entrant blur handler must observe this promise and never dispatch
    // a second rename.
    editor.commitPromise = Promise.resolve()
      .then(() => onRename(editor.index, name, editor.input))
      .catch((error) => {
        showInlineError(
          editor,
          t("无法重命名：{0}", "Unable to rename: {0}", [errorMessage(error)]),
        );
        return "retry" as InlineRenameResult;
      });
    editor.input.disabled = true;
    editor.input.setAttribute("aria-busy", "true");
    const result = await editor.commitPromise;
    if (activeEditor !== editor) return true;

    if (result === "refresh") {
      await cancelInlineRename(editor, true, true);
      await refreshFromFirstPage();
      return true;
    }
    if (result === "retry") {
      editor.commitPromise = null;
      editor.input.disabled = false;
      editor.input.removeAttribute("aria-busy");
      editor.input.focus({ preventScroll: true });
      const currentItem = items[editor.index];
      placeInlineCaret(
        editor.input,
        currentItem?.path_type.endsWith("Dir") ?? false,
      );
      return false;
    }
    if (result !== "succeeded") {
      await cancelInlineRename(editor, true, true);
      return true;
    }

    const current = items[editor.index];
    if (!current || current.name !== editor.sourceName) {
      notifyMutation(MUTATION_EFFECT.OUTCOME_UNKNOWN);
      await cancelInlineRename(editor, true, true);
      return true;
    }
    loadedNames.delete(current.name);
    const renamedPath = replaceLogicalBasename(current.name, name);
    const renamed = Object.freeze({ ...current, name: renamedPath });
    items.splice(editor.index, 1, renamed);
    loadedNames.add(renamedPath);
    activeEditor = null;
    table.classList.remove("has-inline-editor");
    replaceRenderedRow(editor.index);
    if (shouldRestoreNameFocus) {
      focusName(editor.index);
    } else {
      restoreBlurFocus(editor);
    }
    await refreshAfterInlineRename(editor, renamedPath);
    return true;
  }

  async function cancelInlineRename(
    editor: InlineEditor,
    restoreFocus: boolean,
    keepInvalidated: boolean = false,
  ) {
    if (activeEditor !== editor) return;
    activeEditor = null;
    table.classList.remove("has-inline-editor");
    replaceRenderedRow(editor.index);
    if (restoreFocus) {
      const currentRename = document.querySelector('[data-file-action="rename"]');
      focusElement(
        editor.returnFocus?.isConnected
          ? editor.returnFocus
          : currentRename || nameLink(editor.index),
      );
    } else {
      restoreBlurFocus(editor);
    }
    if (keepInvalidated) {
      refreshAfterEditor = false;
    } else if (editor.created || refreshAfterEditor) {
      await refreshAfterInlineRename(editor, editor.sourceName);
    }
  }

  async function refreshAfterInlineRename(
    editor: InlineEditor,
    finalName: string,
  ) {
    refreshAfterEditor = false;
    await refreshFromFirstPage();
    if (editor.created && params.q && !matchesFilter(finalName, params.q)) {
      listStatus.textContent = t(
        "已创建“{0}”，但当前筛选条件将其隐藏。",
        'Created "{0}", but it is hidden by the current filter.',
        [logicalBasename(finalName)],
      );
    }
  }

  function showInlineError(editor: InlineEditor, message: string) {
    if (activeEditor !== editor) return;
    editor.error.textContent = message;
    editor.error.hidden = false;
    editor.input.setAttribute("aria-invalid", "true");
  }

  function clearInlineError(editor: InlineEditor) {
    if (activeEditor !== editor) return;
    editor.error.textContent = "";
    editor.error.hidden = true;
    editor.input.removeAttribute("aria-invalid");
  }

  function replaceRenderedRow(index: number) {
    const row = document.getElementById(`addPath${index}`);
    const item = items[index];
    if (!(row instanceof HTMLTableRowElement) || !item) return;
    row.replaceWith(createPathRow(item, index));
  }

  function focusName(index: number) {
    focusElement(nameLink(index));
  }

  function nameLink(index: number): HTMLAnchorElement | null {
    const link = document.querySelector(`#addPath${index} .cell-name a`);
    return link instanceof HTMLAnchorElement ? link : null;
  }

  function captureBlurFocus(
    index: number,
    target: EventTarget | null,
  ): InlineEditor["blurFocus"] {
    if (!(target instanceof Element)) return null;
    const row = document.getElementById(`addPath${index}`);
    if (!(row instanceof HTMLTableRowElement) || !row.contains(target)) {
      return { element: target };
    }
    return { control: "name" };
  }

  function restoreBlurFocus(editor: InlineEditor) {
    const target = editor.blurFocus;
    if (!target) return;
    if ("element" in target) {
      focusElement(target.element);
      return;
    }
    const row = document.getElementById(`addPath${editor.index}`);
    const selector = ".cell-name a";
    focusElement(row?.querySelector(selector));
  }

  function addPath(
    file: ListingItem,
    index: number,
    destination: HTMLElement | DocumentFragment = tableBody,
  ) {
    destination.append(createPathRow(file, index));
  }

  function reindexPathRow(row: HTMLTableRowElement, index: number) {
    row.id = `addPath${index}`;
    row.dataset.index = String(index);
  }

  function pathRowUsesIndex(row: HTMLTableRowElement, index: number) {
    return row.id === `addPath${index}` && row.dataset.index === String(index);
  }

  function createPathRow(
    file: ListingItem,
    index: number,
  ): HTMLTableRowElement {
    let url = childUrl(file.name);
    const isDir =
      typeof file.path_type === "string" && file.path_type.endsWith("Dir");
    if (isDir) url += "/";

    const row = createElement("tr", {
      attributes: { id: `addPath${index}`, "data-index": index },
    }) as HTMLTableRowElement;
    row.classList.toggle("is-tag-selected", selectedTagName === file.name);
    const iconCell = createElement("td", {
      className: "path cell-icon",
    });
    iconCell.append(getPathIcon(file.path_type));
    const nameCell = createElement("td", {
      className: "path cell-name",
    });
    nameCell.append(
      createElement("a", {
        text: file.name,
        attributes: {
          href: url,
          download: isDir ? false : true,
          "aria-describedby": fileActionState().action ? "file-action-hint" : undefined,
        },
      }),
    );

    const tagCell = createElement("td", {
      className: "cell-tags",
      attributes: { "data-label": t("标签", "Tags") },
    });
    if (file.tagsAvailable === false)
      tagCell.append(
        createElement("span", { text: t("标签暂不可用", "Tags unavailable") }),
      );
    for (const tag of file.tags ?? []) {
      const label = createElement("span", {
        className: "file-tag-label",
        text: tag.name,
      });
      if (tag.color) label.style.borderInlineStartColor = tag.color;
      tagCell.append(label);
    }
    if (!tagCell.childNodes.length) tagCell.textContent = "—";

    row.append(
      iconCell,
      nameCell,
      tagCell,
      createElement("td", {
        className: "cell-mtime",
        text: formatMtime(file.mtime),
      }),
      createElement("td", {
        className: "cell-size",
        text: isDir ? "" : formatFileSize(file.size).join(" "),
      }),
    );
    return row;
  }

  function setupActions() {
    const updateMode = () => {
      const { action } = fileActionState();
      table.dataset.actionMode = action ?? "";
      if (action !== "tags") selectedTagName = null;
      for (const row of tableBody.rows) {
        const link = row.querySelector(".cell-name a");
        if (action) link?.setAttribute("aria-describedby", "file-action-hint");
        else link?.removeAttribute("aria-describedby");
        row.classList.toggle("is-tag-selected", items[Number(row.dataset.index)]?.name === selectedTagName);
      }
    };
    subscribeFileAction(updateMode);
    updateMode();
    tableBody.addEventListener("click", event => {
      const { action } = fileActionState();
      const target = event.target;
      if (!action || !(target instanceof Element) || target.closest("input, button, select, textarea")) return;
      const row = target.closest("tr[data-index]");
      if (!(row instanceof HTMLTableRowElement) || !tableBody.contains(row)) return;
      event.preventDefault();
      const item = items[Number(row.dataset.index)];
      if (!item) return;
      if ((action === "download" && item.path_type.endsWith("Dir")) ||
          (action === "tags" && item.path_type !== "File")) {
        if (action === "tags") {
          selectedTagName = null;
          window.dispatchEvent(new CustomEvent("xczs:file-tags", { detail: null }));
        }
        selectFileAction(action, action === "tags"
          ? t("请选择普通文件，文件夹不支持标签。", "Choose a regular file. Folders cannot have tags.")
          : t("请选择文件，文件夹不支持下载。", "Choose a file. Folders cannot be downloaded."));
        return;
      }
      void chooseTarget(action, item.name);
    });
    tableBody.addEventListener("keydown", event => {
      if (event.key === " " && !event.isComposing && fileActionState().action &&
          event.target instanceof HTMLAnchorElement && event.target.closest(".cell-name")) {
        event.preventDefault();
        event.target.click();
      }
    });
    window.addEventListener("keydown", event => {
      if (event.key === "Escape" && !event.isComposing && fileActionState().action) {
        event.preventDefault();
        selectFileAction(null);
      }
    });
  }

  async function chooseTarget(action: FileAction, name: string) {
    if (choosing) return;
    choosing = true;
    try {
      if (!(await settleInlineRename()) || fileActionState().action !== action) return;
      const index = items.findIndex(item => item?.name === name);
      const file = items[index];
      if (!file) return;
      if (action === "tags") {
        selectedTagName = file.name;
        selectFileAction("tags");
        for (const row of tableBody.rows)
          row.classList.toggle("is-tag-selected", Number(row.dataset.index) === index);
        onTags(file);
        return;
      }
      selectFileAction(null);
      if (action === "rename") {
        await startInlineRename(index, document.querySelector('[data-file-action="rename"]'));
      } else if (action === "download") {
        nameLink(index)?.click();
      } else {
        onAction(action, index);
      }
    } finally { choosing = false; }
  }

  async function runListingControl(action: () => void | Promise<void>) {
    if (await settleInlineRename()) await action();
  }

  function remove(index: number) {
    const row = document.getElementById(`addPath${index}`);
    const focused = document.activeElement;
    const moveFocus =
      focused instanceof Node && Boolean(row?.contains(focused));
    const focusTarget =
      row?.nextElementSibling?.querySelector("button, a") ||
      row?.previousElementSibling?.querySelector("button, a") ||
      (document.getElementById("search") as HTMLElement | null);
    row?.remove();
    const removed = items[index];
    if (!removed) return;
    items[index] = null;
    loadedNames.delete(removed.name);
    visibleCount--;
    if (index >= renderedStart && index < renderedEnd) {
      renderedVisibleCount = Math.max(0, renderedVisibleCount - 1);
    }
    if (visibleCount === 0 && nextCursor !== null) {
      void loadNextPage();
    } else {
      updateVisibility();
    }
    if (moveFocus && focusTarget instanceof HTMLElement) focusTarget.focus();
  }

  function removeByName(name: string) {
    const index = items.findIndex((item) => item?.name === name);
    if (index >= 0) remove(index);
  }

  renderHead();
  setupActions();
  loadMore.addEventListener("click", () => {
    void runListingControl(loadNextPage);
  });
  showPrevious.addEventListener("click", () => {
    void runListingControl(showPreviousWindow);
  });
  showNext.addEventListener("click", () => {
    void runListingControl(showNextWindow);
  });

  return Object.freeze({
    getItem(index: number) {
      return items[index] || null;
    },
    addCreatedItem,
    loadNextPage,
    notifyMutation,
    refreshFromFirstPage,
    remove,
    removeByName,
    settleInlineRename,
    startInlineRename,
    showEmpty() {
      loaded = true;
      nextCursor = null;
      updateVisibility();
    },
  });
}

function validateCreatedItem(file: ListingItem): Readonly<ListingItem> {
  if (
    !file ||
    !PATH_TYPES.has(file.path_type) ||
    !isValidLogicalPath(file.name) ||
    !Number.isSafeInteger(file.mtime) ||
    file.mtime < 0 ||
    !Number.isSafeInteger(file.size) ||
    file.size < 0 ||
    !isCanonicalRevision(file.revision)
  ) {
    throw new TypeError(
      t("新建文件列表项目无效", "Invalid created file list item"),
    );
  }
  return Object.freeze({ ...file });
}

function isValidInlineName(name: string) {
  return isValidLogicalPath(name) && !name.includes("/");
}

function logicalBasename(path: string) {
  return path.slice(path.lastIndexOf("/") + 1);
}

function replaceLogicalBasename(path: string, name: string) {
  const separator = path.lastIndexOf("/");
  return separator < 0 ? name : `${path.slice(0, separator + 1)}${name}`;
}

function selectionEnd(name: string, isDirectory: boolean) {
  if (isDirectory) return name.length;
  const separator = name.lastIndexOf(".");
  return separator > 0 ? separator : name.length;
}

function placeInlineCaret(input: HTMLInputElement, isDirectory: boolean) {
  const position = selectionEnd(input.value, isDirectory);
  input.setSelectionRange(position, position);
}

function matchesFilter(name: string, query: string) {
  return name.toLowerCase().includes(query.toLowerCase());
}

function focusElement(element: Element | null | undefined) {
  if (element instanceof HTMLElement && element.isConnected) {
    element.focus({ preventScroll: true });
  }
}

function validateListingPage(payload: unknown): {
  paths: readonly ListingItem[];
  nextCursor: string | null;
} {
  if (!payload || typeof payload !== "object") {
    throw new Error(t("文件列表响应无效", "Invalid file list response"));
  }
  const page = payload as Record<string, unknown>;
  const nextCursor = page.next_cursor;
  if (
    !Array.isArray(page.paths) ||
    page.paths.length > LIST_PAGE_LIMIT ||
    !(
      nextCursor === null ||
      (typeof nextCursor === "string" &&
        nextCursor.length > 0 &&
        nextCursor.length <= MAX_CURSOR_LENGTH)
    )
  ) {
    throw new Error(t("文件列表响应无效", "Invalid file list response"));
  }
  const names = new Set();
  if (
    page.file_tags !== undefined &&
    page.file_tags !== null &&
    (!Array.isArray(page.file_tags) ||
      page.file_tags.length !== page.paths.length ||
      !page.file_tags.every(listedTags))
  )
    throw new Error(t("文件标签响应无效", "Invalid file tag response"));
  const metadata = Array.isArray(page.file_tags) ? page.file_tags : null;
  const paths = page.paths.map((candidate, index) => {
    if (!candidate || typeof candidate !== "object") {
      throw new Error(t("文件列表项目无效", "Invalid file list item"));
    }
    const file = candidate as Record<string, unknown>;
    if (
      typeof file.path_type !== "string" ||
      !PATH_TYPES.has(file.path_type) ||
      typeof file.name !== "string" ||
      !isValidLogicalPath(file.name) ||
      file.name.length > 32_768 ||
      typeof file.mtime !== "number" ||
      !Number.isSafeInteger(file.mtime) ||
      file.mtime < 0 ||
      typeof file.size !== "number" ||
      !Number.isSafeInteger(file.size) ||
      file.size < 0 ||
      !isCanonicalRevision(file.revision) ||
      names.has(file.name)
    ) {
      throw new Error(t("文件列表项目无效", "Invalid file list item"));
    }
    names.add(file.name);
    return Object.freeze({
      path_type: file.path_type as ListingItem["path_type"],
      name: file.name,
      mtime: file.mtime,
      size: file.size,
      revision: file.revision as string,
      tags: metadata?.[index]?.tags ?? [],
      fileId: metadata?.[index]?.file_id ?? null,
      tagsHasMore: metadata?.[index]?.tags_has_more ?? false,
      tagsAvailable: page.file_tags !== null,
    });
  });
  return Object.freeze({
    paths: Object.freeze(paths),
    nextCursor,
  });
}

function isCanonicalRevision(value: unknown) {
  return typeof value === "string" && /^[0-9a-f]{64}$/.test(value);
}

function createWindowButton(text: string) {
  return createElement("button", {
    className: "load-more hidden",
    text,
    attributes: { type: "button" },
  });
}

function getPathIcon(pathType: ListingItem["path_type"]) {
  switch (pathType) {
    case "Dir":
      return createIcon("dir");
    case "SymlinkFile":
      return createIcon("symlinkFile");
    case "SymlinkDir":
      return createIcon("symlinkDir");
    default:
      return createIcon("file");
  }
}

function formatMtime(mtime: number) {
  if (!mtime) return "";
  const date = new Date(mtime);
  const year = date.getUTCFullYear();
  const month = padZero(date.getUTCMonth() + 1, 2);
  const day = padZero(date.getUTCDate(), 2);
  const hours = padZero(date.getUTCHours(), 2);
  const minutes = padZero(date.getUTCMinutes(), 2);
  return `${year}-${month}-${day} ${hours}:${minutes} UTC`;
}

function padZero(value: number, size: number) {
  return ("0".repeat(size) + value).slice(-size);
}
