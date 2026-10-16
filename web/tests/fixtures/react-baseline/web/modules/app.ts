import { t } from "../platform.ts";
import { createActionDialogs } from "./operations/dialogs.ts";
import { createFileOperations } from "./operations/file_operations.ts";
import { createDirectoryListing } from "./listing/controller.ts";
import { createElement, createIcon } from "./shared/dom.ts";
import { ApiClientError, mountFileWorkspace } from "../platform.ts";
import {
  administratorApi,
  authenticationErrorMessage,
} from "./platform-session.ts";
// React owns the page; product controllers own the file/queue/dialog regions.
import { parseIndexData } from "./shared/index_data.ts";
import { currentPageUrl } from "./shared/path.ts";
import { createUploadManager } from "./upload/manager.ts";
import { workspaceView } from "../react/navigation.ts";

let data: ReturnType<typeof parseIndexData>;

let directoryListing: ReturnType<typeof createDirectoryListing>;

let fileOperations: ReturnType<typeof createFileOperations>;

let uploadManager: ReturnType<typeof createUploadManager>;
let redirectingToLogin = false;

const searchParams = new URLSearchParams(window.location.search);
const requestedSort = searchParams.get("sort") || "";
const requestedOrder = searchParams.get("order") || "";
const params = {
  q: searchParams.get("q") || "",
  sort: ["name", "mtime", "size"].includes(requestedSort)
    ? requestedSort
    : "name",
  order:
    ["name", "mtime", "size"].includes(requestedSort) &&
    ["asc", "desc"].includes(requestedOrder)
      ? requestedOrder
      : "asc",
  all: searchParams.get("all") || "",
  any: searchParams.get("any") || "",
  exclude: searchParams.get("exclude") || "",
};

export function start() {
  const initializePage = () => {
    void initialize().catch((error) => {
      if (error instanceof ApiClientError && error.status === 401) {
        location.replace("/__xczs__/login");
        return;
      }
      showFatalError(authenticationErrorMessage(error));
    });
  };
  if (document.readyState === "loading")
    window.addEventListener("DOMContentLoaded", initializePage, { once: true });
  else initializePage();
}

async function initialize() {
  const container = document.getElementById("xczs-root");
  if (!container) throw new Error("Xczs application root is missing");
  const indexData = document.getElementById(
    "index-data",
  ) as HTMLTemplateElement | null;
  if (!indexData) throw new Error(t("页面数据缺失", "Page data is missing"));

  const rawData: unknown = JSON.parse(
    decodeBase64(indexData.content.textContent || ""),
  );
  data = parseIndexData(rawData, await administratorApi.restore());
  mountFileWorkspace(container, administratorApi, data.href.slice(1));
  addBreadcrumb(data.href);
  const updateTitle = () => {
    const view = workspaceView();
    document.title =
      view === "tags" || view === "status"
        ? t("xczs 标签库", "xczs Tag Library")
        : t("{0} - xczs 文件管理", "{0} - xczs File Manager", [data.href]);
  };
  updateTitle();

  const pathsTable = requiredElement(".paths-table", HTMLTableElement);
  const pathsTableBody = requiredElement(
    ".paths-table tbody",
    HTMLTableSectionElement,
  );
  const emptyFolder = requiredElement(".empty-folder", HTMLElement);
  const dialogs = createActionDialogs();
  const emptyNote = data.dir_exists
    ? t("文件夹为空", "Folder is empty")
    : t(
        "上传文件后将自动创建此文件夹",
        "Uploading files will create this folder automatically",
      );

  directoryListing = createDirectoryListing({
    data,
    params,
    table: pathsTable,
    tableHead: requiredElement(".paths-table thead", HTMLTableSectionElement),
    tableBody: pathsTableBody,
    emptyFolder,
    emptyNote,
    loadMore: requiredElement(".load-more", HTMLButtonElement),
    listStatus: requiredElement(".list-status", HTMLElement),
    onAction(action, index) {
      if (action === "move") {
        void fileOperations.movePath(index);
      } else if (action === "delete") {
        void fileOperations.deletePath(index);
      }
    },
    onRename(index, name, returnFocus) {
      return fileOperations.renamePath(index, name, returnFocus);
    },
    onUnauthorized: redirectToLogin,
    onTags(file) {
      const path = `${data.href === "/" ? "" : data.href.slice(1) + "/"}${file.name}`;
      window.dispatchEvent(
        new CustomEvent("xczs:file-tags", {
          detail: { name: file.name, path, fileId: file.fileId ?? null },
        }),
      );
    },
  });
  fileOperations = createFileOperations({
    data,
    listing: directoryListing,
    dialogs,
    operationStatus: requiredElement(".operation-status", HTMLElement),
    onUnauthorized: redirectToLogin,
  });
  uploadManager = createUploadManager({
    data,
    dialogs,
    uploadersTable: requiredElement(".uploaders-table", HTMLTableElement),
    queueMessage: requiredElement(".upload-queue-message", HTMLElement),
    historyStatus: requiredElement(".upload-history-status", HTMLElement),
    emptyFolder,
    onMutation: (effect) => directoryListing.notifyMutation(effect),
    onUnauthorized: redirectToLogin,
  });

  setupFileDropGuard();
  setupUploadFile();
  setupUploadFolder();
  setupNewFolder();
  setupNewFile();
  setupAuth();
  setupSearch();

  requiredElement(".index-page", HTMLElement).classList.remove("hidden");
  let listingStarted = false;
  const showFiles = async () => {
    if (listingStarted || workspaceView() !== "files") return;
    listingStarted = true;
    if (data.dir_exists) await directoryListing.loadNextPage();
    else directoryListing.showEmpty();
  };
  const changed = () => {
    updateTitle();
    void showFiles().catch((error) =>
      showFatalError(authenticationErrorMessage(error)),
    );
  };
  window.addEventListener("hashchange", changed);
  window.addEventListener("popstate", changed);
  window.addEventListener("xczs:tags-changed", () => {
    if (listingStarted) void directoryListing.refreshFromFirstPage();
  });
  await showFiles();
}

function requiredElement<T extends Element>(
  selector: string,
  constructor: { new (...args: never[]): T },
): T {
  const element = document.querySelector(selector);
  if (!element)
    throw new Error(
      t("缺少必要的页面控件：{0}", "Required page control is missing: {0}", [
        selector,
      ]),
    );
  if (constructor && !(element instanceof constructor)) {
    throw new Error(
      t(
        "必要的页面控件类型不正确：{0}",
        "Required page control has the wrong type: {0}",
        [selector],
      ),
    );
  }
  return element as T;
}

function showFatalError(message: string) {
  document.querySelector(".index-page")?.classList.remove("hidden");
  const status = document.querySelector(".workspace-status");
  if (status) {
    status.classList.remove("hidden");
    status.textContent = message;
    return;
  }
  document.body.append(
    createElement("p", {
      text: message,
      attributes: { role: "alert" },
    }),
  );
}

function addBreadcrumb(href: string) {
  const rootNavigation = requiredElement(".xczs-root-navigation", HTMLElement);
  const breadcrumb = requiredElement(".breadcrumb", HTMLElement);
  const root = createElement("a", {
    attributes: {
      href: "/",
      title: t("根目录", "Root"),
      "aria-label": t("根目录", "Root"),
      ...(href === "/" ? { "aria-current": "page" } : {}),
    },
  });
  root.append(createIcon("home"));
  rootNavigation.append(root);
  breadcrumb.title = href;
  if (href === "/") {
    breadcrumb.append(
      createElement("b", {
        text: "/",
        attributes: { "aria-current": "page" },
      }),
    );
    return;
  }
  breadcrumb.append(
    createElement("a", { text: "/", attributes: { href: "/" } }),
  );
  const parts = href.slice(1).split("/");
  let path = "/";
  for (let index = 0; index < parts.length; index++) {
    const name = parts[index];
    path += encodeURIComponent(name);
    if (index === parts.length - 1) {
      breadcrumb.append(
        createElement("b", {
          text: name,
          attributes: { "aria-current": "page" },
        }),
      );
    } else {
      breadcrumb.append(
        createElement("a", {
          text: name,
          attributes: { href: path },
        }),
      );
    }
    if (index !== parts.length - 1) {
      path += "/";
      breadcrumb.append(
        createElement("span", {
          className: "separator",
          text: "/",
          attributes: { "aria-hidden": "true" },
        }),
      );
    }
  }
}

function setupFileDropGuard() {
  for (const name of ["dragover", "drop"]) {
    document.addEventListener(name, (event) => {
      const dataTransfer = (event as DragEvent).dataTransfer;
      const types = Array.from(dataTransfer?.types || []);
      if (!dataTransfer || !types.includes("Files")) return;
      event.preventDefault();
      dataTransfer.dropEffect = "none";
    });
  }
}

function setupAuth() {
  const logout = requiredElement(".logout-btn", HTMLButtonElement);
  requiredElement(".user-name", HTMLElement).textContent =
    data.session.username;
  logout.classList.remove("hidden");
  logout.addEventListener("click", () => {
    void fileOperations.logout();
  });
}

function setupSearch() {
  const searchbar = requiredElement(".searchbar", HTMLFormElement);
  searchbar.classList.remove("hidden");
  searchbar.addEventListener("submit", (event) => {
    event.preventDefault();
    const form = new FormData(searchbar);
    void applyFilters(
      String(form.get("q") || ""),
      Object.fromEntries(
        ["all", "any", "exclude"].map((key) => [
          key,
          form.getAll(key).join(","),
        ]),
      ),
    );
  });
  if (params.q) {
    requiredElement("#search", HTMLInputElement).value = params.q;
  }
  window.addEventListener("xczs:clear-tag-filters", () => {
    void applyFilters(requiredElement("#search", HTMLInputElement).value, {
      all: "",
      any: "",
      exclude: "",
    });
  });
  window.addEventListener("popstate", () => {
    const query = new URLSearchParams(location.search);
    const next = Object.fromEntries(
      ["q", "all", "any", "exclude", "sort", "order"].map((key) => [
        key,
        query.get(key) ||
          (key === "sort" ? "name" : key === "order" ? "asc" : ""),
      ]),
    );
    if (
      Object.entries(next).some(
        ([key, value]) => value !== params[key as keyof typeof params],
      )
    ) {
      Object.assign(params, next);
      requiredElement("#search", HTMLInputElement).value = params.q;
      void directoryListing.refreshFromFirstPage();
    }
  });
}

async function applyFilters(query: string, tags: Record<string, string>) {
  if (!(await directoryListing.settleInlineRename())) return;
  params.q = query;
  Object.assign(params, tags);
  const url = new URL(currentPageUrl());
  for (const [key, value] of Object.entries(params))
    if (value) url.searchParams.set(key, value);
  url.hash = "files";
  window.history.pushState(null, "", url);
  // pushState does not emit hashchange; update the existing workspace as well.
  window.dispatchEvent(new PopStateEvent("popstate"));
  const details = document.querySelector(".search-tags");
  if (details instanceof HTMLDetailsElement) details.open = false;
  await directoryListing.refreshFromFirstPage();
}

function setupUploadFile() {
  const button = requiredElement(".upload-file", HTMLButtonElement);
  const input = requiredElement("#file", HTMLInputElement);
  button.classList.remove("hidden");
  button.addEventListener("click", () => input.click());
  input.addEventListener("change", () => {
    void uploadManager.addFiles(input.files, { returnFocus: button });
    input.value = "";
  });
}

function setupUploadFolder() {
  const button = requiredElement(".upload-folder", HTMLButtonElement);
  const input = requiredElement("#folder", HTMLInputElement);
  button.classList.remove("hidden");
  button.addEventListener("click", () => input.click());
  input.addEventListener("change", () => {
    void uploadManager.addFiles(input.files, { returnFocus: button });
    input.value = "";
  });
}

function setupNewFolder() {
  const button = requiredElement(".new-folder", HTMLButtonElement);
  button.classList.remove("hidden");
  button.addEventListener("click", () => {
    void fileOperations.createDefaultFolder(button);
  });
}

function setupNewFile() {
  const button = requiredElement(".new-file", HTMLButtonElement);
  button.classList.remove("hidden");
  button.addEventListener("click", () => {
    void fileOperations.createDefaultFile(button);
  });
}

function redirectToLogin() {
  if (redirectingToLogin) return;
  redirectingToLogin = true;
  // A running upload can raise a beforeunload confirmation. If the user
  // cancels that navigation, this document remains active and must be able to
  // react to a later authentication failure. A successful reload destroys the
  // document before this reset matters.
  window.setTimeout(() => {
    redirectingToLogin = false;
  }, 0);
  location.reload();
}

function decodeBase64(base64String: string) {
  const binary = atob(base64String);
  const bytes = Uint8Array.from(binary, (character) => character.charCodeAt(0));
  return new TextDecoder().decode(bytes);
}
