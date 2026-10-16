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

export type ActionDialogs = import("./dialogs.ts").ActionDialogs;

export type FileOperationsOptions = {
  data: IndexData;
  listing: {
    getItem: (
      index: number,
    ) => { name: string; path_type: string; revision: string } | null;
    addCreatedItem: (file: {
      path_type: "Dir" | "File";
      name: string;
      mtime: number;
      size: number;
      revision: string;
    }) => number;
    remove: (index: number) => void;
    removeByName: (name: string) => void;
    notifyMutation: (
      effect: (typeof MUTATION_EFFECT)[keyof typeof MUTATION_EFFECT],
    ) => boolean;
    refreshFromFirstPage: () => Promise<void>;
    settleInlineRename: () => Promise<boolean>;
    startInlineRename: (
      index: number,
      returnFocus?: Element | null,
      options?: { created?: boolean },
    ) => Promise<boolean>;
  };
  dialogs: ActionDialogs;
  operationStatus: HTMLElement;
  onUnauthorized: () => void;
};

import { t } from "../../platform.ts";
import {
  CSRF_HEADER,
  OPERATION_ID_HEADER,
  RequestError,
  assertDiscardUploadResponse,
  assertFreshUploadResponse,
  assertResponse,
  isAuthenticationError,
  postJson,
  queryUnknownUpload,
  requestJson,
  requestNoContent,
  runMutationWithReconciliation,
} from "../http/client.ts";
import { errorMessage } from "../shared/dom.ts";
import { MUTATION_EFFECT } from "../shared/mutation_effect.ts";
import {
  browserUrlFromLogicalPath,
  childUrl,
  isValidLogicalPath,
  logicalChildPath,
} from "../shared/path.ts";
import {
  UPLOAD_ID_HEADER,
  UPLOAD_LENGTH_HEADER,
  UPLOAD_OVERWRITE_HEADER,
} from "../upload/protocol.ts";
import { parseUploadPreflight } from "../upload/preflight.ts";

const DEFAULT_FOLDER_NAME = "newfolder";
const DEFAULT_FILE_NAME = "newfile";
const DEFAULT_NAME_ATTEMPT_LIMIT = 1_000;
const REVISION_REFRESH_CODES = Object.freeze([
  "delete_target_changed",
  "source_changed",
  "destination_changed",
]);

const ERROR_CODE_STATUS: Readonly<Record<string, number>> = Object.freeze({
  path_exists: 409,
  destination_exists: 409,
  delete_target_changed: 412,
  source_changed: 412,
  destination_changed: 412,
});

export function createFileOperations(options: FileOperationsOptions) {
  const { data, listing, dialogs, operationStatus, onUnauthorized } = options;

  const pending: Map<string, { control: Element | null; message: string }> =
    new Map();

  async function deletePath(index: number) {
    const file = listing.getItem(index);
    if (!file || !isValidLogicalPath(file.name)) return;
    const source = logicalPath(file.name);
    const pendingKey = `path:${source}`;
    if (pending.has(pendingKey)) return;
    const returnFocus = document.querySelector('[data-file-action="delete"]');
    if (
      !(await dialogs.confirmAction({
        title: t("删除项目", "Delete item"),
        message: t(
          "删除“{0}”？此操作无法撤销。",
          'Delete "{0}"? This action cannot be undone.',
          [file.name],
        ),
        confirmText: t("删除", "Delete"),
        danger: true,
        returnFocus,
      }))
    )
      return;
    if (
      !begin(
        pendingKey,
        returnFocus,
        t("正在删除 {0}…", "Deleting {0}…", [file.name]),
      )
    )
      return;
    try {
      const operationId = crypto.randomUUID();
      const result = await runMutationWithReconciliation(
        () =>
          requestNoContent(
            childUrl(file.name),
            {
              method: "DELETE",
              headers: {
                [CSRF_HEADER]: data.session.csrf_token,
                [OPERATION_ID_HEADER]: operationId,
                "If-Match": `"${file.revision}"`,
              },
            },
            {
              outcomeUnknown: true,
              operationId,
            },
          ),
        onUnauthorized,
      );
      const effect = trackedMutationEffect(result);
      listing.notifyMutation(effect);
      if (result.kind === "authentication") return;
      if (result.kind === "succeeded") {
        listing.removeByName(file.name);
        await listing.refreshFromFirstPage();
        return;
      }
      await dialogs.showMessage({
        title: t("删除失败", "Delete failed"),
        message: t("无法删除“{0}”：{1}", 'Unable to delete "{0}": {1}', [
          file.name,
          result.status?.message || errorMessage(result.error),
        ]),
        returnFocus,
      });
      if (effect === MUTATION_EFFECT.REFRESH_REQUIRED) {
        await listing.refreshFromFirstPage();
      }
    } finally {
      end(pendingKey);
    }
  }

  async function renamePath(
    index: number,
    name: string,
    returnFocus: Element | null,
  ): Promise<InlineRenameResult> {
    const file = listing.getItem(index);
    if (!file || !isValidLogicalPath(file.name)) return "unknown";
    const source = logicalPath(file.name);
    const currentName = logicalBasename(source);
    const pendingKey = `path:${source}`;
    if (pending.has(pendingKey)) return "retry";
    if (name === currentName) return "succeeded";
    if (!isValidLogicalPath(name) || name.includes("/")) {
      return "retry";
    }
    const destination = logicalChild(logicalParent(source), name);
    return await relocatePath({
      endpoint: "rename",
      request: {
        source,
        name,
        source_revision: file.revision,
        overwrite: false,
      },
      source,
      destination,
      fileName: file.name,
      returnFocus,
      verb: "rename",
      onSuccess: () => {},
    });
  }

  async function movePath(index: number) {
    const file = listing.getItem(index);
    if (!file || !isValidLogicalPath(file.name)) return;
    const source = logicalPath(file.name);
    const pendingKey = `path:${source}`;
    if (pending.has(pendingKey)) return;
    const sourceName = logicalBasename(source);
    const currentDirectory = logicalParent(source);
    const returnFocus = document.querySelector('[data-file-action="move"]');
    let directory = await dialogs.requestText({
      title: t("移动项目", "Move item"),
      message: t(
        "请输入共享根目录下已有的目标文件夹，项目名称将保持不变。",
        "Enter an existing destination folder under the shared root. The item name will not change.",
      ),
      label: t("目标文件夹", "Destination folder"),
      value: currentDirectory,
      confirmText: t("移动", "Move"),
      returnFocus,
    });
    if (directory === null || directory === "") return;
    if (!directory.startsWith("/")) directory = `/${directory}`;
    while (directory.length > 1 && directory.endsWith("/")) {
      directory = directory.slice(0, -1);
    }
    if (directory !== "/" && !isValidLogicalPath(directory.slice(1))) {
      await dialogs.showMessage({
        title: t("移动失败", "Move failed"),
        message: t(
          "目标文件夹路径无效。",
          "The destination folder path is invalid.",
        ),
        returnFocus,
      });
      return;
    }
    const destination = logicalChild(directory, sourceName);
    if (source === destination) return;
    await relocatePath({
      endpoint: "move",
      request: {
        source,
        directory,
        source_revision: file.revision,
        overwrite: false,
      },
      source,
      destination,
      fileName: file.name,
      returnFocus,
      verb: "move",
      onSuccess() {
        location.href = browserUrlFromLogicalPath(directory);
      },
    });
  }

  async function relocatePath(options: {
    endpoint: "move" | "rename";
    request:
      | {
          source: string;
          directory: string;
          source_revision: string;
          overwrite: boolean;
          destination_revision?: string;
        }
      | {
          source: string;
          name: string;
          source_revision: string;
          overwrite: boolean;
          destination_revision?: string;
        };
    source: string;
    destination: string;
    fileName: string;
    returnFocus: Element | null;
    verb: "move" | "rename";
    onSuccess: () => void | Promise<void>;
  }): Promise<InlineRenameResult> {
    const {
      endpoint,
      request,
      source,
      destination,
      fileName,
      returnFocus,
      verb,
      onSuccess,
    } = options;
    const pendingKey = `path:${source}`;
    const presentParticiple =
      verb === "rename" ? t("正在重命名", "Renaming") : t("正在移动", "Moving");
    const titleVerb =
      verb === "rename" ? t("重命名", "Rename") : t("移动", "Move");
    if (!begin(pendingKey, returnFocus, `${presentParticiple} ${fileName}…`)) {
      return "retry";
    }

    try {
      let result = await runMutationWithReconciliation(
        () => postBrowserApi(endpoint, request),
        onUnauthorized,
      );
      if (result.kind === "authentication") return "authentication";
      const destinationRevision = trackedConflictRevision(
        result,
        "destination_exists",
        "targetRevision",
      );
      if (destinationRevision) {
        if (
          !(await dialogs.confirmAction({
            title: t("覆盖目标？", "Overwrite destination?"),
            message: t("用“{1}”替换“{0}”？", 'Replace "{0}" with "{1}"?', [
              destination,
              source,
            ]),
            confirmText: t("覆盖", "Overwrite"),
            danger: true,
            returnFocus,
          }))
        )
          return "retry";
        const overwriteRequest = {
          ...request,
          overwrite: true,
          destination_revision: destinationRevision,
        };
        result = await runMutationWithReconciliation(
          () => postBrowserApi(endpoint, overwriteRequest),
          onUnauthorized,
        );
        if (result.kind === "authentication") return "authentication";
      }
      const effect = trackedMutationEffect(result);
      listing.notifyMutation(effect);
      if (result.kind !== "succeeded") {
        await dialogs.showMessage({
          title: t("{0}失败", "{0} failed", [titleVerb]),
          message: t(
            "无法将“{1}”{0}到“{2}”：{3}",
            'Unable to {0} "{1}" to "{2}": {3}',
            [
              verb === "rename" ? t("重命名", "rename") : t("移动", "move"),
              source,
              destination,
              result.status?.message || errorMessage(result.error),
            ],
          ),
          returnFocus,
        });
        if (effect === MUTATION_EFFECT.REFRESH_REQUIRED) {
          if (verb === "rename") return "refresh";
          await listing.refreshFromFirstPage();
          return "retry";
        }
        return effect === MUTATION_EFFECT.OUTCOME_UNKNOWN ? "unknown" : "retry";
      }
      await onSuccess();
      return "succeeded";
    } catch (error) {
      if (isAuthenticationError(error)) return "authentication";
      listing.notifyMutation(MUTATION_EFFECT.OUTCOME_UNKNOWN);
      await dialogs.showMessage({
        title: t("{0}失败", "{0} failed", [titleVerb]),
        message: t(
          "无法将“{1}”{0}到“{2}”：{3}",
          'Unable to {0} "{1}" to "{2}": {3}',
          [
            verb === "rename" ? t("重命名", "rename") : t("移动", "move"),
            source,
            destination,
            errorMessage(error),
          ],
        ),
        returnFocus,
      });
      return "unknown";
    } finally {
      end(pendingKey);
    }
  }

  async function logout() {
    const returnFocus = document.querySelector(".logout-btn");
    if (!begin("logout", returnFocus, t("正在退出…", "Signing out…"))) return;
    try {
      const { administratorApi } = await import("../platform-session.ts");
      await administratorApi.logout();
      onUnauthorized();
    } catch (error) {
      if (isAuthenticationError(error)) return;
      await dialogs.showMessage({
        title: t("退出失败", "Sign out failed"),
        message: t(
          "无法退出，请重试。",
          "Unable to sign out. Please try again.",
        ),
        returnFocus,
      });
    } finally {
      end("logout");
    }
  }

  async function createDefaultFolder(returnFocus: Element | null) {
    if (!(await listing.settleInlineRename())) return;
    if (
      !begin(
        "create-item",
        returnFocus,
        t("正在创建文件夹…", "Creating a new folder…"),
      )
    )
      return;
    try {
      for (let attempt = 0; attempt < DEFAULT_NAME_ATTEMPT_LIMIT; attempt++) {
        const name = defaultCandidate(DEFAULT_FOLDER_NAME, attempt);
        const result = await runMutationWithReconciliation(
          () => postBrowserApi("mkdir", { path: logicalPath(name) }),
          onUnauthorized,
        );
        if (result.kind === "authentication") return;
        if (result.kind === "succeeded") {
          listing.notifyMutation(MUTATION_EFFECT.COMMITTED);
          await showCreatedItem(name, "Dir", returnFocus);
          return;
        }
        if (isDefiniteTrackedConflict(result, "path_exists")) continue;

        listing.notifyMutation(trackedMutationEffect(result));
        await dialogs.showMessage({
          title: t("创建文件夹失败", "Create folder failed"),
          message: t(
            "无法创建文件夹“{0}”：{1}",
            'Unable to create folder "{0}": {1}',
            [name, result.status?.message || errorMessage(result.error)],
          ),
          returnFocus,
        });
        return;
      }
      await showDefaultNameLimit("folder", returnFocus);
    } finally {
      end("create-item");
    }
  }

  async function createDefaultFile(returnFocus: Element | null) {
    if (!(await listing.settleInlineRename())) return;
    if (
      !begin(
        "create-item",
        returnFocus,
        t("正在创建空文件…", "Creating a new empty file…"),
      )
    )
      return;
    try {
      for (let attempt = 0; attempt < DEFAULT_NAME_ATTEMPT_LIMIT; attempt++) {
        const name = defaultCandidate(DEFAULT_FILE_NAME, attempt);
        const uploadId = crypto.randomUUID();
        const targetUrl = childUrl(name);
        try {
          const response = await requestNoContent(
            targetUrl,
            {
              method: "PUT",
              headers: {
                [CSRF_HEADER]: data.session.csrf_token,
                [UPLOAD_ID_HEADER]: uploadId,
                [UPLOAD_LENGTH_HEADER]: "0",
                [UPLOAD_OVERWRITE_HEADER]: "false",
              },
              body: "",
            },
            {
              outcomeUnknown: true,
              resultId: uploadId,
            },
          );
          await assertFreshUploadResponse(
            response,
            uploadId,
            0,
            onUnauthorized,
          );
          listing.notifyMutation(MUTATION_EFFECT.COMMITTED);
          await showCreatedItem(name, "File", returnFocus);
          return;
        } catch (error) {
          if (isAuthenticationError(error)) return;
          if (isDefiniteFreshUploadConflict(error)) continue;
          if (isAwaitingUploadConflict(error, uploadId)) {
            if (await discardStagedUpload(name, uploadId, returnFocus))
              continue;
            return;
          }

          const status = await queryUnknownUpload(
            error,
            targetUrl,
            0,
            onUnauthorized,
          );
          if (status?.authenticationFailed) return;
          if (status?.state === "committed") {
            listing.notifyMutation(MUTATION_EFFECT.COMMITTED);
            await showCreatedItem(name, "File", returnFocus);
            return;
          }
          if (status?.state === "awaiting-confirmation") {
            if (await discardStagedUpload(name, uploadId, returnFocus))
              continue;
            return;
          }

          listing.notifyMutation(uploadMutationEffect(error, status));
          await dialogs.showMessage({
            title: t("创建文件失败", "Create file failed"),
            message: t(
              "无法创建文件“{0}”：{1}",
              'Unable to create file "{0}": {1}',
              [name, status?.message || errorMessage(error)],
            ),
            returnFocus,
          });
          return;
        }
      }
      await showDefaultNameLimit("file", returnFocus);
    } finally {
      end("create-item");
    }
  }

  async function showCreatedItem(
    name: string,
    pathType: "Dir" | "File",
    returnFocus: Element | null,
  ) {
    const path = logicalPath(name);
    let revision;
    try {
      const { response, payload } = await requestJson(
        "/__xczs__/api/upload/preflight",
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            [CSRF_HEADER]: data.session.csrf_token,
          },
          body: JSON.stringify({ paths: [path] }),
        },
        { outcomeUnknown: false },
      );
      await assertResponse(response, onUnauthorized);
      const [target] = parseUploadPreflight(payload, [path]);
      revision = target.exists ? target.revision : null;
    } catch (error) {
      if (isAuthenticationError(error)) return;
      await listing.refreshFromFirstPage();
      return;
    }
    if (revision === null) {
      await listing.refreshFromFirstPage();
      return;
    }
    const index = listing.addCreatedItem({
      path_type: pathType,
      name,
      mtime: Date.now(),
      size: 0,
      revision,
    });
    await listing.startInlineRename(index, returnFocus, { created: true });
  }

  /** A staged conflict owns server-side temporary state. Only the bound 204 +
   * rejected upload envelope proves that state was discarded; every other
   * result stops candidate generation so a later upload ID cannot leak or
   * bypass the staged session.
   */
  async function discardStagedUpload(
    name: string,
    uploadId: string,
    returnFocus: Element | null,
  ) {
    try {
      const response = await requestNoContent(
        "/__xczs__/api/upload/discard",
        {
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            [CSRF_HEADER]: data.session.csrf_token,
          },
          body: JSON.stringify({
            path: logicalPath(name),
            upload_id: uploadId,
          }),
        },
        {
          outcomeUnknown: true,
          resultId: uploadId,
        },
      );
      await assertDiscardUploadResponse(response, uploadId, 0, onUnauthorized);
      return true;
    } catch (error) {
      if (isAuthenticationError(error)) return false;
      listing.notifyMutation(MUTATION_EFFECT.OUTCOME_UNKNOWN);
      await dialogs.showMessage({
        title: t("创建文件失败", "Create file failed"),
        message: t(
          "无法确认暂存文件“{0}”的清理结果：{1}",
          'Unable to confirm cleanup for staged file "{0}": {1}',
          [name, errorMessage(error)],
        ),
        returnFocus,
      });
      return false;
    }
  }

  async function showDefaultNameLimit(
    kind: "file" | "folder",
    returnFocus: Element | null,
  ) {
    await dialogs.showMessage({
      title:
        kind === "file"
          ? t("创建文件失败", "Create file failed")
          : t("创建文件夹失败", "Create folder failed"),
      message: t(
        "尝试 {1} 次后仍未找到可用的默认{0}名称。",
        "No available default {0} name was found after {1} attempts.",
        [
          kind === "file" ? t("文件", "file") : t("文件夹", "folder"),
          DEFAULT_NAME_ATTEMPT_LIMIT,
        ],
      ),
      returnFocus,
    });
  }

  function begin(key: string, control: Element | null, message: string) {
    if (pending.has(key)) return false;
    pending.set(key, { control, message });
    setControlBusy(control, true);
    renderOperationStatus();
    return true;
  }

  function end(key: string) {
    const operation = pending.get(key);
    if (!operation) return;
    pending.delete(key);
    setControlBusy(operation.control, [...pending.values()].some(value => value.control === operation.control));
    renderOperationStatus();
  }

  function renderOperationStatus() {
    const latest = Array.from(pending.values()).at(-1);
    operationStatus.textContent = latest?.message || "";
    operationStatus.classList.toggle("hidden", !latest);
  }

  function logicalPath(name: string) {
    return logicalChildPath(data.href, name);
  }

  function logicalBasename(path: string) {
    return path.slice(path.lastIndexOf("/") + 1);
  }

  function logicalParent(path: string) {
    const separator = path.lastIndexOf("/");
    return separator <= 0 ? "/" : path.slice(0, separator);
  }

  function logicalChild(parent: string, name: string) {
    return parent === "/" ? `/${name}` : `${parent}/${name}`;
  }

  function postBrowserApi(action: string, body: unknown) {
    return postJson(`/__xczs__/api/${action}`, data.session.csrf_token, body);
  }

  return Object.freeze({
    createDefaultFile,
    createDefaultFolder,
    deletePath,
    logout,
    movePath,
    renamePath,
  });
}

function defaultCandidate(base: string, attempt: number) {
  return attempt === 0 ? base : `${base} (${attempt + 1})`;
}

function isDefiniteTrackedConflict(
  result: Awaited<ReturnType<typeof runMutationWithReconciliation>>,
  code: string,
) {
  const expectedStatus = ERROR_CODE_STATUS[code];
  if (!expectedStatus) return false;
  if (result.kind !== "failed") return false;
  if (result.status?.kind === "failed") {
    return (
      result.status.code === code && result.status.status === expectedStatus
    );
  }
  return (
    result.status === null &&
    result.error instanceof RequestError &&
    result.error.code === code &&
    result.error.status === expectedStatus &&
    (!result.error.problemStatus ||
      result.error.problemStatus === expectedStatus) &&
    !result.error.outcomeUnknown &&
    (!result.error.operationState ||
      ["failed", "rejected"].includes(result.error.operationState))
  );
}

/** Return conditional authority only from the direct, definite failure that
 * carried it. A reconciled job result cannot safely recreate response headers.
 */
function trackedConflictRevision(
  result: Awaited<ReturnType<typeof runMutationWithReconciliation>>,
  code: string,
  field: "sourceRevision" | "targetRevision",
) {
  return isDefiniteTrackedConflict(result, code) &&
    result.kind === "failed" &&
    result.status === null &&
    result.error instanceof RequestError
    ? result.error[field]
    : "";
}

function isDefiniteFreshUploadConflict(error: unknown) {
  return (
    error instanceof RequestError &&
    error.code === "destination_exists" &&
    error.status === ERROR_CODE_STATUS.destination_exists &&
    (!error.problemStatus ||
      error.problemStatus === ERROR_CODE_STATUS.destination_exists) &&
    !error.outcomeUnknown &&
    ["not-started", "rejected"].includes(
      error.uploadState || error.operationState,
    )
  );
}

function isAwaitingUploadConflict(error: unknown, uploadId: string) {
  return (
    error instanceof RequestError &&
    !error.outcomeUnknown &&
    error.uploadId === uploadId &&
    error.uploadLength === 0 &&
    error.uploadState === "awaiting-confirmation"
  );
}

/** Convert the reconciled tracked-operation result into the single listing
 * invalidation protocol. A terminal failed job proves that no successful
 * mutation remains to observe, even if the original transport error was
 * conservative. Running, unknown, or unavailable reconciliation cannot make
 * that guarantee.
 */
export function trackedMutationEffect(
  result: Awaited<ReturnType<typeof runMutationWithReconciliation>>,
): (typeof MUTATION_EFFECT)[keyof typeof MUTATION_EFFECT] {
  if (result.kind === "succeeded") return MUTATION_EFFECT.COMMITTED;
  if (result.kind !== "failed") return MUTATION_EFFECT.NOT_COMMITTED;
  if (
    REVISION_REFRESH_CODES.some((code) =>
      isDefiniteTrackedConflict(result, code),
    )
  ) {
    return MUTATION_EFFECT.REFRESH_REQUIRED;
  }
  if (
    result.status?.kind === "running" ||
    result.status?.kind === "unknown" ||
    result.status?.kind === "unavailable"
  ) {
    return MUTATION_EFFECT.OUTCOME_UNKNOWN;
  }
  return result.status === null &&
    result.error instanceof RequestError &&
    result.error.outcomeUnknown
    ? MUTATION_EFFECT.OUTCOME_UNKNOWN
    : MUTATION_EFFECT.NOT_COMMITTED;
}

/** Classify an empty-file PUT after its one status reconciliation. Only a
 * matching terminal rejection suppresses invalidation: `not-seen` can race a
 * detached original request that has not persisted its session yet, so it
 * cannot prove that the target will remain unchanged.
 */
export function uploadMutationEffect(
  error: unknown,
  status: Awaited<ReturnType<typeof queryUnknownUpload>>,
): (typeof MUTATION_EFFECT)[keyof typeof MUTATION_EFFECT] {
  if (status?.state === "committed") return MUTATION_EFFECT.COMMITTED;
  if (status?.state === "rejected") return MUTATION_EFFECT.NOT_COMMITTED;
  if (status?.state === "running" || status?.state === "unknown") {
    return MUTATION_EFFECT.OUTCOME_UNKNOWN;
  }
  return error instanceof RequestError && error.outcomeUnknown
    ? MUTATION_EFFECT.OUTCOME_UNKNOWN
    : MUTATION_EFFECT.NOT_COMMITTED;
}

function setControlBusy(control: Element | null | undefined, busy: boolean) {
  if (!control || typeof control.setAttribute !== "function") return;
  if (busy) {
    control.setAttribute("aria-busy", "true");
    control.setAttribute("aria-disabled", "true");
  } else {
    control.removeAttribute("aria-busy");
    control.removeAttribute("aria-disabled");
  }
}
