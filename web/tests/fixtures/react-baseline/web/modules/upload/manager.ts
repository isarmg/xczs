export type UploadLifecycleState =
  | "new"
  | "queued"
  | "running"
  | "completed"
  | "failed"
  | "unknown"
  | "cancelled";

export type UploadPhase =
  | "new"
  | "transferring"
  | "checking"
  | "submitting"
  | "awaiting-confirmation"
  | "completed"
  | "failed"
  | "unknown"
  | "cancelled";

export type UploadRecoveryAction = "" | "retry" | "query_upload";

export type DialogChoice = "confirm" | "alternate" | "cancel";

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
  max_concurrent_uploads?: number;
};

export type UploadView = {
  row: HTMLTableRowElement;
  statusCell: HTMLTableCellElement;
  speedNode: HTMLSpanElement;
  progressNode: HTMLSpanElement;
  liveNode: HTMLSpanElement;
  cancelButton: HTMLButtonElement;
};

export type UploadSelectionEntry =
  import("./selection.ts").UploadSelectionEntry;

export type QueuedUploadEntry = {
  file: File;
  name: string;
  revision: string | null;
};

export type UploadManagerOptions = {
  data: IndexData;
  dialogs: {
    showMessage: (options: {
      title: string;
      message?: string;
      returnFocus?: Element | null;
    }) => Promise<undefined>;
    chooseAction: (options: {
      title: string;
      message?: string;
      confirmText?: string;
      alternateText?: string;
      cancelText?: string;
      danger?: boolean;
      returnFocus?: Element | null;
    }) => Promise<DialogChoice>;
  };
  uploadersTable: HTMLTableElement;
  queueMessage: HTMLElement;
  historyStatus: HTMLElement;
  emptyFolder: HTMLElement;
  onMutation: (
    effect: (typeof MUTATION_EFFECT)[keyof typeof MUTATION_EFFECT],
  ) => void;
  onUnauthorized: () => void;
  maxConcurrentUploads?: number;
  maxTerminalRows?: number;
};

export type TerminalUploadRow = {
  row: HTMLTableRowElement;
  name: string;
  dispose: (() => void) | null;
};

export type PreflightReservation = { count: number; active: boolean };

import { t } from "../../platform.ts";
import { createElement, errorMessage, formatFileSize } from "../shared/dom.ts";
import { MUTATION_EFFECT } from "../shared/mutation_effect.ts";
import { platformErrorCode } from "../http/platform-error.ts";
import {
  AUTH_REQUIRED_MESSAGE,
  CSRF_HEADER,
  ERROR_RESPONSE_BODY_LIMIT,
  PAGE_EXPIRED_MESSAGE,
  RESULT_UNKNOWN_MESSAGE,
  assertDiscardUploadResponse,
  assertResponse,
  isAuthenticationError,
  isRequestErrorCode,
  parseErrorPayload,
  requestHead,
  requestJson,
  requestNoContent,
  responsePlatformErrorCode,
} from "../http/client.ts";
import {
  childUrl,
  isValidAbsoluteLogicalPath,
  logicalChildPath,
} from "../shared/path.ts";
import {
  TARGET_REVISION_HEADER,
  UPLOAD_ID_HEADER,
  UPLOAD_LENGTH_HEADER,
  UPLOAD_OFFSET_HEADER,
  UPLOAD_OVERWRITE_HEADER,
  classifyUploadResponse,
  parseTargetReplaceable,
  parseTargetRevision,
} from "./protocol.ts";
import { parseUploadPreflight } from "./preflight.ts";
import { createBoundedHistory, createUploadQueue } from "./queue.ts";
import {
  UPLOAD_BATCH_PATH_BYTES_LIMIT,
  prepareUploadSelection,
} from "./selection.ts";
import { createUploadRequest, dispatchUploadRequest } from "./transport.ts";
import {
  createUploadView,
  renderCancelled,
  renderCheckpoint,
  renderCleanup,
  renderComplete,
  renderFailure,
  renderProgress,
  renderSkipped,
  renderSubmitting,
  renderUnknown,
  renderWaitingForOverwrite,
  renderWaiting,
} from "./view.ts";

const DEFAULT_MAX_CONCURRENT_UPLOADS = 1;
const MAX_CLIENT_UPLOAD_CONCURRENCY = 8;
const ENQUEUE_BATCH_SIZE = 50;
const IDLE_TIMEOUT_MS = 2 * 60 * 1000;
const TOTAL_TIMEOUT_MS = 24 * 60 * 60 * 1000;
const STATUS_TIMEOUT_MS = 30 * 1000;
const COMMIT_TIMEOUT_MS = 5 * 60 * 1000;
const MAX_TIMER_DELAY_MS = 2_147_483_647;
export const UPLOAD_PENDING_ROW_LIMIT = 512;
export const UPLOAD_TERMINAL_ROW_LIMIT = 200;
const UPLOAD_RECOVERY_LABELS = Object.freeze({
  retry: t("重试上传", "Retry upload"),
  query_upload: t("核对上传状态", "Check upload status"),
});
const UNKNOWN_UPLOAD_RESULT_MESSAGE = t(
  "上传数据已发送，但服务器未确认结果。{0}",
  "Upload data was sent, but the server did not confirm the result. {0}",
  [RESULT_UNKNOWN_MESSAGE],
);

export function createUploadManager(options: UploadManagerOptions) {
  const {
    data,
    dialogs,
    uploadersTable,
    queueMessage,
    historyStatus,
    emptyFolder,
    onMutation,
    onUnauthorized,
  } = options;
  const maxConcurrentUploads = normalizeConcurrency(
    options.maxConcurrentUploads ?? data.max_concurrent_uploads,
  );
  const maxTerminalRows = normalizeTerminalRowLimit(options.maxTerminalRows);

  const queue: ReturnType<typeof createUploadQueue> = createUploadQueue();

  const failed: Map<number, Uploader> = new Map();

  const unresolvedUnknown: Set<number> = new Set();

  const knownTargets: Set<string> = new Set();
  let running = 0;
  let pendingRows = 0;
  let preflightReservedRows = 0;
  let nextIndex = 0;
  let nextBatchId = 0;

  const batches: Map<
    number,
    {
      members: Set<Uploader>;
      enqueueComplete: boolean;
      cancelRequested: boolean;
      remainingEntries: number;
    }
  > = new Map();
  let cancellingBatch = false;

  let queueState: "running" | "paused-auth" | "paused-unknown" = "running";

  let enqueueTail: Promise<void> = Promise.resolve();
  let restoreHistoryFocusAfterRender = false;
  const terminalHistory = createBoundedHistory(
    maxTerminalRows,
    (entry: TerminalUploadRow) => {
      const restoreFocus = entry.row.contains(document.activeElement);
      const adjacentNameLink = restoreFocus
        ? adjacentUploadNameLink(entry.row)
        : null;
      entry.dispose?.();
      entry.row.remove();
      if (!restoreFocus) return;
      if (adjacentNameLink?.isConnected) {
        adjacentNameLink.focus({ preventScroll: true });
      } else {
        restoreHistoryFocusAfterRender = true;
      }
    },
  );

  const beforeUnload = (event: BeforeUnloadEvent) => {
    if (
      running > 0 ||
      (queueState === "running" &&
        (preflightReservedRows > 0 || queue.size > 0))
    ) {
      event.preventDefault();
      event.returnValue = "";
      return "";
    }
  };
  window.addEventListener("beforeunload", beforeUnload);

  function retainTerminalRow(
    uploader: Uploader,
    dispose: (() => void) | null = null,
  ) {
    if (uploader.pendingAccounted) {
      uploader.pendingAccounted = false;
      pendingRows = Math.max(0, pendingRows - 1);
    }
    if (uploader.terminalHistoryEntry) {
      terminalHistory.remove(uploader.terminalHistoryEntry);
    }
    const entry = {
      row: uploader.view.row,
      name: uploader.name,
      dispose,
    };
    knownTargets.delete(uploader.name);
    uploader.terminalHistoryEntry = entry;
    const batch = batches.get(uploader.batchId);
    batch?.members.delete(uploader);
    if (batch?.enqueueComplete && batch.members.size === 0) {
      batches.delete(uploader.batchId);
    }
    terminalHistory.add(entry);
    renderHistoryStatus();
  }

  function renderHistoryStatus() {
    const hiddenCount = terminalHistory.evicted;
    historyStatus.textContent =
      hiddenCount > 0
        ? t(
            "已隐藏 {0} 条较早的上传结果；",
            "{0} older upload result{1} hidden; ",
            [hiddenCount, hiddenCount === 1 ? "" : "s"],
          ) +
          t("显示最近 {0} 条。", "showing the most recent {0}.", [
            terminalHistory.size,
          ])
        : "";
    historyStatus.classList.toggle("hidden", hiddenCount === 0);
    if (restoreHistoryFocusAfterRender) {
      restoreHistoryFocusAfterRender = false;
      historyStatus.tabIndex = -1;
      historyStatus.focus({ preventScroll: true });
      historyStatus.addEventListener(
        "blur",
        () => historyStatus.removeAttribute("tabindex"),
        { once: true },
      );
    }
  }

  function hasPendingCapacity(additionalRows: number = 1) {
    return (
      pendingRows + preflightReservedRows + additionalRows <=
      UPLOAD_PENDING_ROW_LIMIT
    );
  }

  function reservePreflightRows(count: number): PreflightReservation | null {
    if (!hasPendingCapacity(count)) return null;
    preflightReservedRows += count;
    return { count, active: true };
  }

  function releasePreflightRows(reservation: PreflightReservation | null) {
    if (!reservation?.active) return;
    if (preflightReservedRows < reservation.count) {
      throw new Error(
        t(
          "上传预留计数下溢",
          "Upload preflight reservation accounting underflowed",
        ),
      );
    }
    preflightReservedRows -= reservation.count;
    reservation.active = false;
  }

  function transferPreflightRows(
    reservation: PreflightReservation | null,
    admittedRows: number,
  ) {
    if (
      !reservation?.active ||
      !Number.isSafeInteger(admittedRows) ||
      admittedRows < 1 ||
      admittedRows > reservation.count ||
      preflightReservedRows < reservation.count
    ) {
      throw new Error(
        t("上传预留无效", "Upload preflight reservation is invalid"),
      );
    }
    const remainingReserved = preflightReservedRows - reservation.count;
    if (
      pendingRows + remainingReserved + admittedRows >
      UPLOAD_PENDING_ROW_LIMIT
    ) {
      return false;
    }
    preflightReservedRows = remainingReserved;
    pendingRows += admittedRows;
    reservation.active = false;
    return true;
  }

  function showPendingLimitMessage() {
    queueMessage.textContent =
      t(
        "最多同时等待 {0} 个上传。",
        "At most {0} uploads may be pending at once. ",
        [UPLOAD_PENDING_ROW_LIMIT],
      ) +
      t(
        "请等待上传完成后再添加或重试。",
        "Wait for pending uploads to finish before adding or retrying more.",
      );
    queueMessage.classList.remove("hidden");
  }

  function releaseTerminalRow(uploader: Uploader) {
    if (!uploader.terminalHistoryEntry) return;
    terminalHistory.remove(uploader.terminalHistoryEntry);
    uploader.terminalHistoryEntry = null;
    renderHistoryStatus();
  }

  function pauseForAuthentication() {
    if (queueState === "paused-auth") return;
    queueState = "paused-auth";
    // Other in-flight requests are not aborted when dispatch is paused.
    // Keep their unload protection if navigation is requested or cancelled.
    queueMessage.textContent = PAGE_EXPIRED_MESSAGE;
    queueMessage.classList.remove("hidden");
  }

  function pauseForUnknown(uploader: Uploader) {
    unresolvedUnknown.add(uploader.index);
    if (queueState !== "paused-auth") queueState = "paused-unknown";
    // Pausing dispatch does not stop other concurrent transfers. Keep their
    // unload protection until they finish; a paused queue alone may refresh.
    queueMessage.textContent = t(
      "{0} 的上传结果不确定，剩余队列已暂停。请刷新文件夹后再选择文件。",
      "Upload result for {0} is unknown. The remaining upload queue is paused; refresh the folder before selecting files again.",
      [uploader.name],
    );
    queueMessage.classList.remove("hidden");
  }

  function resolveUnknown(uploader: Uploader) {
    if (!unresolvedUnknown.delete(uploader.index)) return;
    if (unresolvedUnknown.size > 0 || queueState !== "paused-unknown") return;
    queueState = "running";
    window.addEventListener("beforeunload", beforeUnload);
    queueMessage.classList.add("hidden");
    runQueue();
  }

  function runQueue() {
    if (queueState !== "running" || cancellingBatch) return;
    while (running < maxConcurrentUploads) {
      const uploader = queue.dequeue() as Uploader | null;
      if (!uploader) return;
      if (uploader.state !== "queued") continue;
      uploader.queueEntry = null;
      running++;
      uploader.runningAccounted = true;
      uploader.start();
    }
  }

  class Uploader {
    name: string;

    index: number;

    file: File;

    url: string;

    logicalPath: string;

    targetRevision: string | null;

    missingTargetRetryUsed: boolean;

    batchId: number;

    uploadId: string;

    uploadOffset: number;

    uploaded: number;

    lastUpdate: number;

    lastProgressAt: number;

    lastAnnouncedProgress: number;
    state: UploadLifecycleState;
    pendingRecovery: UploadRecoveryAction;
    recoveryAction: UploadRecoveryAction;

    recoveryAvailableAt: number;
    recoveryTimer: number | null;

    runningAccounted: boolean;

    pendingAccounted: boolean;
    queueEntry: { active: boolean } | null;
    abortController: AbortController | null;

    abortReason: string;

    abortOutcomeUnknown: boolean;
    idleTimer: number | null;
    totalTimer: number | null;
    commitTimer: number | null;
    terminalHistoryEntry: TerminalUploadRow | null;
    phase: UploadPhase;

    uploadRequestPhase: "fresh" | "resume" | "commit";

    requestDispatched: boolean;

    view: ReturnType<typeof createUploadView>;
    constructor(
      file: File,
      name: string,
      targetRevision: string | null,
      batchId: number,
    ) {
      this.index = nextIndex++;
      this.file = file;
      this.name = name;
      this.url = childUrl(this.name);
      this.logicalPath = logicalChildPath(data.href, this.name);
      this.targetRevision = targetRevision;
      this.missingTargetRetryUsed = false;
      this.batchId = batchId;
      this.uploadId = crypto.randomUUID();
      this.uploadOffset = 0;
      this.uploaded = 0;
      this.lastUpdate = 0;
      this.lastProgressAt = 0;
      this.lastAnnouncedProgress = -1;

      this.state = "new";

      this.pendingRecovery = "";

      this.recoveryAction = "";
      this.recoveryAvailableAt = 0;

      this.recoveryTimer = null;
      this.runningAccounted = false;
      this.pendingAccounted = false;

      this.queueEntry = null;

      this.abortController = null;
      this.abortReason = "";
      this.abortOutcomeUnknown = false;

      this.idleTimer = null;

      this.totalTimer = null;

      this.commitTimer = null;

      this.terminalHistoryEntry = null;

      this.phase = "new";
      this.uploadRequestPhase = "fresh";
      this.requestDispatched = false;
      this.view = createUploadView(this.index, this.name, this.url, () =>
        this.cancel(),
      );
    }

    enqueue(recovery: UploadRecoveryAction = "") {
      releaseTerminalRow(this);
      if (!this.pendingAccounted) {
        this.pendingAccounted = true;
        pendingRows++;
      }
      if (!this.view.row.isConnected) {
        (uploadersTable.tBodies[0] || uploadersTable.createTBody()).append(
          this.view.row,
        );
      }
      uploadersTable.classList.remove("hidden");
      emptyFolder.classList.add("hidden");
      this.state = "queued";
      this.pendingRecovery = recovery;
      renderWaiting(this.view, this.name, Boolean(recovery));
      this.queueEntry = queue.enqueue(this);
      runQueue();
    }

    start() {
      this.state = "running";
      const recovery = this.pendingRecovery;
      this.pendingRecovery = "";
      switch (recovery) {
        case "retry":
          void this.queryCheckpoint(false, true);
          break;
        case "query_upload":
          void this.queryCheckpoint();
          break;
        default:
          this.sendBody();
      }
    }

    sendBody(forceResume = false) {
      this.uploaded = 0;
      this.lastUpdate = Date.now();
      this.lastProgressAt = this.lastUpdate;
      this.lastAnnouncedProgress = 0;
      this.phase = "transferring";
      this.requestDispatched = false;
      const request = createUploadRequest({
        responseLimit: ERROR_RESPONSE_BODY_LIMIT,
        onProgress: (event: ProgressEvent) => this.onProgress(event),
        onBodySent: () => this.beginCommitWait(),
        onResponse: (response: XMLHttpRequest) =>
          this.handleUploadResponse(response),
        onNetworkError: () => this.handleNetworkError(),
        onAbort: () => this.handleAbort(),
        onOversizedResponse: () => this.handleOversizedResponse(),
      });
      this.startTimeouts(() => request.abort());
      renderProgress(
        this.view,
        this.name,
        t("等待数据", "Waiting for data"),
        "0% --:--:--",
        true,
      );

      try {
        const resuming = forceResume || this.uploadOffset > 0;
        this.uploadRequestPhase = resuming ? "resume" : "fresh";
        const commonHeaders = {
          [CSRF_HEADER]: data.session.csrf_token,
          [UPLOAD_ID_HEADER]: this.uploadId,
          [UPLOAD_LENGTH_HEADER]: String(this.file.size),
          [UPLOAD_OVERWRITE_HEADER]: String(this.targetRevision !== null),
          ...(this.targetRevision === null
            ? {}
            : { [TARGET_REVISION_HEADER]: this.targetRevision }),
        };
        const headers = resuming
          ? {
              ...commonHeaders,
              [UPLOAD_OFFSET_HEADER]: String(this.uploadOffset),
            }
          : commonHeaders;
        this.requestDispatched = true;
        dispatchUploadRequest(request, {
          method: resuming ? "PATCH" : "PUT",
          url: this.url,
          headers,
          body: resuming ? this.file.slice(this.uploadOffset) : this.file,
        });
      } catch (error) {
        this.requestDispatched = false;
        this.clearTimeouts();
        this.fail(errorMessage(error), "retry");
      }
    }

    handleUploadResponse(request: XMLHttpRequest) {
      this.clearTimeouts();
      const classification = classifyUploadResponse({
        phase: this.uploadRequestPhase === "resume" ? "resume" : "fresh",
        errorCode:
          request.status === 403
            ? platformErrorCode(
                request.responseText,
                request.getResponseHeader("Content-Type"),
              )
            : null,
        status: request.status,
        headers: (name) => request.getResponseHeader(name),
        expectedUploadId: this.uploadId,
        expectedLength: this.file.size,
      });
      if (classification.kind === "committed") {
        this.complete();
        return;
      }

      if (classification.kind === "authentication") {
        pauseForAuthentication();
        this.fail(AUTH_REQUIRED_MESSAGE);
        onUnauthorized();
        return;
      }
      if (classification.kind === "csrf") {
        pauseForAuthentication();
        this.fail(PAGE_EXPIRED_MESSAGE);
        return;
      }
      const detail = parseErrorPayload(
        request.responseText,
        request.getResponseHeader("Content-Type") || "",
      );
      if (detail.status && detail.status !== request.status) {
        this.unknown(
          t(
            "错误响应无效：问题状态与 HTTP 状态不一致。{0}",
            "Invalid error response: problem status does not match HTTP status. {0}",
            [RESULT_UNKNOWN_MESSAGE],
          ),
          detail.recovery === "query_upload" ? "query_upload" : "",
          responseRetryAfter(request, detail.retryAfter),
        );
        return;
      }
      const targetChange = trustedUploadTargetChange(
        classification,
        detail,
        (name) => request.getResponseHeader(name),
        this.file.size,
        this.uploadRequestPhase === "resume" ? this.uploadOffset : null,
      );
      if (targetChange) this.invalidateTargetChange();
      if (targetChange?.kind === "missing") {
        this.retryMissingTarget(
          classification.kind === "awaiting-confirmation",
        );
        return;
      }
      if (targetChange?.kind === "reset-stage") {
        this.retryMissingTarget(true);
        return;
      }
      if (targetChange?.kind === "exists") {
        const staged = classification.kind === "awaiting-confirmation";
        if (!targetChange.replaceable) {
          if (staged) {
            void this.discardStaged(
              t("目标无法替换", "destination cannot be replaced"),
            );
          } else {
            this.skipConflict(
              t("目标无法替换", "destination cannot be replaced"),
            );
          }
          return;
        }
        void this.resolveDestinationConflict(targetChange.revision, staged);
        return;
      }
      if (
        classification.kind === "awaiting-confirmation" &&
        !classification.outcomeUnknown &&
        classification.protocol?.length === this.file.size &&
        classification.protocol?.offset === this.file.size &&
        detail.status === request.status &&
        detail.recovery === "query_upload"
      ) {
        void this.queryCheckpoint();
        return;
      }
      if (classification.kind === "unknown") {
        this.unknown(
          detail.message
            ? `${detail.message}. ${RESULT_UNKNOWN_MESSAGE}`
            : UNKNOWN_UPLOAD_RESULT_MESSAGE,
          detail.recovery === "query_upload" ? "query_upload" : "",
          responseRetryAfter(request, detail.retryAfter),
        );
        return;
      }
      const knownFailureMessage = uploadFailureMessage(classification.kind);
      if (knownFailureMessage) {
        const recovery = uploadRecoveryAction(
          detail.recovery,
          classification.kind,
          classification.protocol,
        );
        this.fail(
          detail.message || knownFailureMessage,
          recovery,
          responseRetryAfter(request, detail.retryAfter),
        );
        return;
      }
      this.unknown(
        t(
          "服务器返回的上传响应不一致（HTTP {0}）。{1}",
          "The server returned an inconsistent upload response (HTTP {0}). {1}",
          [request.status, RESULT_UNKNOWN_MESSAGE],
        ),
        detail.recovery === "query_upload" ? "query_upload" : "",
        responseRetryAfter(request, detail.retryAfter),
      );
    }

    invalidateTargetChange() {
      if (this.state !== "running") return;
      onMutation(MUTATION_EFFECT.REFRESH_REQUIRED);
    }

    async resolveDestinationConflict(revision: string, staged: boolean) {
      if (this.state !== "running") return;
      this.phase = "awaiting-confirmation";
      renderWaitingForOverwrite(this.view, this.name);
      const choice = await dialogs.chooseAction({
        title: t("上传目标已变更", "Upload destination changed"),
        message: staged
          ? t(
              "上传“{0}”的数据时，目标已存在或发生变化。数据已暂存，请选择覆盖当前目标、跳过此文件或取消剩余队列。",
              '"{0}" now exists or changed while its data was uploaded. The uploaded data is staged; overwrite the current destination, skip this file, or cancel the remaining queued files.',
              [this.name],
            )
          : t(
              "发送“{0}”的数据前，目标已存在或发生变化。请选择覆盖当前目标、跳过此文件或取消剩余队列。",
              '"{0}" now exists or changed before its data was sent. Overwrite the current destination, skip this file, or cancel the remaining queued files.',
              [this.name],
            ),
        confirmText: t("覆盖", "Overwrite"),
        alternateText: t("跳过文件", "Skip file"),
        cancelText: t("取消剩余队列", "Cancel remaining"),
        danger: true,
        returnFocus: this.view.row.querySelector("a"),
      });
      if (this.state !== "running" || this.phase !== "awaiting-confirmation") {
        return;
      }
      if (choice === "confirm") {
        this.targetRevision = revision;
        this.missingTargetRetryUsed = false;
        if (staged) {
          this.publishStaged();
        } else {
          this.sendBody(this.uploadRequestPhase === "resume");
        }
        return;
      }
      if (choice === "cancel") this.cancelRemainingBatch();
      if (staged) {
        await this.discardStaged();
      } else {
        this.skipConflict();
      }
    }

    retryMissingTarget(staged: boolean) {
      if (staged) {
        void this.discardStaged(
          t("目标已移除", "destination was removed"),
          true,
        );
        return;
      }
      if (this.missingTargetRetryUsed) {
        this.fail(
          t(
            "上传目标持续变化，请先核对状态再重试",
            "The upload destination kept changing; check its status before trying again",
          ),
          "query_upload",
        );
        return;
      }
      this.missingTargetRetryUsed = true;
      this.targetRevision = null;
      this.sendBody(this.uploadRequestPhase === "resume");
    }

    publishStaged() {
      if (this.targetRevision === null) {
        this.fail(
          t(
            "暂存上传缺少安全的目标修订，请先丢弃再重试",
            "The staged upload has no safe target revision; discard it before retrying",
          ),
          "query_upload",
        );
        return;
      }
      this.uploaded = 0;
      this.lastUpdate = Date.now();
      this.lastProgressAt = this.lastUpdate;
      this.phase = "transferring";
      this.requestDispatched = false;
      this.uploadRequestPhase = "resume";
      const request = createUploadRequest({
        responseLimit: ERROR_RESPONSE_BODY_LIMIT,
        onProgress: () => {},
        onBodySent: () => this.beginCommitWait(),
        onResponse: (response: XMLHttpRequest) =>
          this.handleUploadResponse(response),
        onNetworkError: () => this.handleNetworkError(),
        onAbort: () => this.handleAbort(),
        onOversizedResponse: () => this.handleOversizedResponse(),
      });
      this.startTimeouts(() => request.abort());
      try {
        this.requestDispatched = true;
        dispatchUploadRequest(request, {
          method: "PATCH",
          url: this.url,
          headers: {
            [CSRF_HEADER]: data.session.csrf_token,
            [UPLOAD_ID_HEADER]: this.uploadId,
            [UPLOAD_LENGTH_HEADER]: String(this.file.size),
            [UPLOAD_OFFSET_HEADER]: String(this.file.size),
            [UPLOAD_OVERWRITE_HEADER]: "true",
            [TARGET_REVISION_HEADER]: this.targetRevision,
          },
          body: null,
        });
        this.beginCommitWait();
      } catch (error) {
        this.requestDispatched = false;
        this.clearTimeouts();
        this.fail(errorMessage(error), "query_upload");
      }
    }

    async discardStaged(
      skipReason: string = t("目标已存在", "destination exists"),
      restartAfterDiscard: boolean = false,
    ) {
      this.phase = "checking";
      renderCleanup(this.view, this.name);
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
              path: this.logicalPath,
              upload_id: this.uploadId,
            }),
          },
          {
            timeoutMs: STATUS_TIMEOUT_MS,
            timeoutMessage: t(
              "暂存上传清理超时",
              "Staged-upload cleanup timed out",
            ),
            outcomeUnknown: true,
            resultId: this.uploadId,
          },
        );
        await assertDiscardUploadResponse(
          response,
          this.uploadId,
          this.file.size,
        );
        this.finishDiscard(skipReason, restartAfterDiscard);
      } catch (error) {
        if (isAuthenticationError(error)) {
          pauseForAuthentication();
          const csrfFailed = isRequestErrorCode(error, "auth.csrf_rejected");
          this.fail(csrfFailed ? PAGE_EXPIRED_MESSAGE : AUTH_REQUIRED_MESSAGE);
          if (!csrfFailed) onUnauthorized();
          return;
        }
        await this.reconcileDiscard(skipReason, restartAfterDiscard);
      }
    }

    async reconcileDiscard(skipReason: string, restartAfterDiscard: boolean) {
      try {
        const response = await requestHead(
          this.url,
          {
            headers: { [UPLOAD_ID_HEADER]: this.uploadId },
          },
          {
            timeoutMs: STATUS_TIMEOUT_MS,
            timeoutMessage: t(
              "暂存上传清理核对超时",
              "Staged-upload cleanup check timed out",
            ),
          },
        );
        const classification = classifyUploadResponse({
          phase: "checkpoint",
          errorCode: await responsePlatformErrorCode(response),
          status: response.status,
          headers: response.headers,
          expectedUploadId: this.uploadId,
          expectedLength: this.file.size,
        });
        if (["rejected", "not-seen"].includes(classification.kind)) {
          this.finishDiscard(skipReason, restartAfterDiscard);
          return;
        }
        if (classification.kind === "committed") {
          this.complete();
          return;
        }
        if (classification.kind === "authentication") {
          pauseForAuthentication();
          this.fail(AUTH_REQUIRED_MESSAGE);
          onUnauthorized();
          return;
        }
        if (classification.kind === "csrf") {
          pauseForAuthentication();
          this.fail(PAGE_EXPIRED_MESSAGE);
          return;
        }
      } catch (error) {
        if (isAuthenticationError(error)) {
          pauseForAuthentication();
          const csrfFailed = isRequestErrorCode(error, "auth.csrf_rejected");
          this.fail(csrfFailed ? PAGE_EXPIRED_MESSAGE : AUTH_REQUIRED_MESSAGE);
          if (!csrfFailed) onUnauthorized();
          return;
        }
      }
      this.fail(
        restartAfterDiscard
          ? t(
              "目标未被覆盖，但无法确认暂存上传是否已清理；请先核对状态再重试",
              "The destination was not overwritten, but staged-upload cleanup could not be confirmed; check its status before retrying",
            )
          : t(
              "文件未被覆盖，但无法确认暂存上传是否已清理；服务器将自动使其过期",
              "File was not overwritten, but cleanup of its staged upload could not be confirmed; the server will expire it automatically",
            ),
        restartAfterDiscard ? "retry" : "",
      );
    }

    finishDiscard(skipReason: string, restartAfterDiscard: boolean) {
      if (!restartAfterDiscard) {
        this.skipConflict(skipReason);
        return;
      }
      this.targetRevision = null;
      this.restartSession();
      this.sendBody();
    }

    skipConflict(reason: string = t("目标已存在", "destination exists")) {
      if (this.state !== "running") return;
      this.clearTimeouts();
      this.clearRecovery();
      this.state = "cancelled";
      this.phase = "cancelled";
      knownTargets.delete(this.name);
      renderSkipped(this.view, this.name, reason);
      retainTerminalRow(this);
      this.finishRunning();
    }

    cancelRemainingBatch() {
      const batch = batches.get(this.batchId);
      if (!batch) return;
      let cancelled = batch.cancelRequested ? 0 : batch.remainingEntries;
      batch.cancelRequested = true;
      cancellingBatch = true;
      try {
        for (const uploader of [...batch.members]) {
          if (uploader === this || uploader.state !== "queued") continue;
          uploader.cancel();
          cancelled++;
        }
      } finally {
        cancellingBatch = false;
      }
      if (cancelled > 0) {
        queueMessage.textContent = t(
          "已取消剩余 {0} 个排队上传。进行中的上传未被中断。",
          "Cancelled {0} remaining queued upload{1}. Uploads already in progress were not interrupted.",
          [cancelled, cancelled === 1 ? "" : "s"],
        );
        queueMessage.classList.remove("hidden");
      }
      runQueue();
    }

    handleNetworkError() {
      const outcomeUnknown = this.requestDispatched;
      this.clearTimeouts();
      if (outcomeUnknown) {
        this.unknown(UNKNOWN_UPLOAD_RESULT_MESSAGE, "query_upload");
      } else {
        this.fail(t("网络连接中断", "Network connection lost"), "retry");
      }
    }

    handleAbort() {
      const outcomeUnknown = this.abortOutcomeUnknown || this.requestDispatched;
      const reason =
        this.abortReason ||
        (outcomeUnknown
          ? UNKNOWN_UPLOAD_RESULT_MESSAGE
          : t("上传已取消", "Upload cancelled"));
      this.clearTimeouts();
      if (outcomeUnknown) {
        this.unknown(reason, "query_upload");
      } else {
        this.fail(reason, "retry");
      }
    }

    handleOversizedResponse() {
      if (this.state !== "running") return;
      this.clearTimeouts();
      this.unknown(
        t(
          "服务器响应超过允许的大小",
          "The server response exceeded the allowed size",
        ),
        "query_upload",
      );
    }

    recover(recovery: UploadRecoveryAction) {
      if (
        this.recoveryAction !== recovery ||
        Date.now() < this.recoveryAvailableAt
      ) {
        return;
      }
      const resolvingUnknown = unresolvedUnknown.has(this.index);
      if (
        queueState !== "running" &&
        !(recovery === "query_upload" && resolvingUnknown)
      ) {
        return;
      }
      // A terminal row no longer consumes pending capacity. Reserve its slot
      // before mutating the failure/history state so a rejected recovery stays
      // fully recoverable and cannot push the nonterminal count past the cap.
      if (!this.pendingAccounted && !hasPendingCapacity()) {
        showPendingLimitMessage();
        return;
      }
      if (knownTargets.has(this.name)) {
        queueMessage.textContent = t(
          "已有其他上传以 {0} 为目标，请等待其完成后再重试。",
          "Another upload already targets {0}. Wait for it to finish before retrying this upload.",
          [this.name],
        );
        queueMessage.classList.remove("hidden");
        return;
      }
      if (!failed.delete(this.index)) return;
      knownTargets.add(this.name);
      this.clearRecovery();
      if (resolvingUnknown) {
        releaseTerminalRow(this);
        if (!this.pendingAccounted) {
          this.pendingAccounted = true;
          pendingRows++;
        }
        this.state = "running";
        running++;
        this.runningAccounted = true;
        void this.queryCheckpoint(true);
        return;
      }
      this.enqueue(recovery);
    }

    async queryCheckpoint(queryingUnknown = false, retryAfterCheck = false) {
      const controller = new AbortController();
      this.abortController = controller;
      this.abortReason = "";
      this.abortOutcomeUnknown = false;
      this.phase = "checking";
      this.requestDispatched = false;
      renderCheckpoint(this.view, this.name);
      try {
        const response = await requestHead(
          this.url,
          {
            signal: controller.signal,
            headers: { [UPLOAD_ID_HEADER]: this.uploadId },
          },
          {
            timeoutMs: STATUS_TIMEOUT_MS,
            timeoutMessage: t(
              "续传状态核对超时",
              "Resume status check timed out",
            ),
          },
        );
        const classification = classifyUploadResponse({
          phase: "checkpoint",
          errorCode: await responsePlatformErrorCode(response),
          status: response.status,
          headers: response.headers,
          expectedUploadId: this.uploadId,
          expectedLength: this.file.size,
        });
        if (classification.kind === "authentication") {
          pauseForAuthentication();
          this.fail(AUTH_REQUIRED_MESSAGE);
          onUnauthorized();
          return;
        }
        if (classification.kind === "csrf") {
          pauseForAuthentication();
          this.fail(PAGE_EXPIRED_MESSAGE);
          return;
        }
        if (["not-seen", "rejected"].includes(classification.kind)) {
          resolveUnknown(this);
          if (retryAfterCheck) {
            this.restartSession();
            this.sendBody();
            return;
          }
          this.fail(
            t(
              "上传会话无法续传，请启动新的上传会话",
              "The upload session cannot be resumed; start a new upload session",
            ),
            "retry",
          );
          return;
        }
        if (classification.kind === "committed") {
          resolveUnknown(this);
          this.complete();
          return;
        }
        if (classification.kind === "awaiting-confirmation") {
          const rawRevision = readHeaderValue(
            response.headers,
            TARGET_REVISION_HEADER,
          );
          const revision = parseTargetRevision(response.headers);
          const replaceable = parseTargetReplaceable(response.headers);
          if (
            replaceable === null ||
            (rawRevision === null && !replaceable) ||
            (rawRevision !== null && revision === null) ||
            classification.protocol?.offset !== this.file.size
          ) {
            this.unknown(
              t(
                "服务器返回的覆盖检查点无效。{0}",
                "The server returned an invalid overwrite checkpoint. {0}",
                [RESULT_UNKNOWN_MESSAGE],
              ),
              "query_upload",
            );
            return;
          }
          resolveUnknown(this);
          if (rawRevision === null) {
            this.missingTargetRetryUsed = false;
            this.retryMissingTarget(true);
            return;
          }
          if (revision === null) {
            this.unknown(
              t(
                "服务器返回的覆盖检查点无效。{0}",
                "The server returned an invalid overwrite checkpoint. {0}",
                [RESULT_UNKNOWN_MESSAGE],
              ),
              "query_upload",
            );
            return;
          }
          if (!replaceable) {
            void this.discardStaged(
              t("目标无法替换", "destination cannot be replaced"),
            );
            return;
          }
          void this.resolveDestinationConflict(revision, true);
          return;
        }
        if (classification.kind === "running") {
          const offset = classification.protocol?.offset;
          if (typeof offset !== "number" || !Number.isSafeInteger(offset)) {
            this.unknown(
              t(
                "服务器返回的上传检查点不一致。{0}",
                "The server returned an inconsistent upload checkpoint. {0}",
                [RESULT_UNKNOWN_MESSAGE],
              ),
              "query_upload",
            );
            return;
          }
          this.uploadOffset = offset;
          if (this.uploadOffset === this.file.size) {
            resolveUnknown(this);
            this.sendBody(true);
            return;
          }
          resolveUnknown(this);
          if (retryAfterCheck) {
            this.sendBody(true);
            return;
          }
          this.fail(
            t(
              "上传仍可从已确认的检查点续传",
              "Upload remains resumable from the confirmed checkpoint",
            ),
            "retry",
          );
          return;
        }
        if (classification.kind === "unknown") {
          if ([429, 503].includes(response.status)) {
            this.unknown(
              response.status === 429
                ? t(
                    "上传状态查询繁忙，请再次核对状态后再上传",
                    "Upload status queries are temporarily busy. Check the upload status again before trying to upload",
                  )
                : t(
                    "上传状态暂不可用，请再次核对状态后再上传",
                    "Upload status is temporarily unavailable. Check the upload status again before trying to upload",
                  ),
              "query_upload",
              response.headers.get("Retry-After"),
            );
            return;
          }
          this.unknown(
            t(
              "服务器记录的发布结果不确定。请刷新文件夹核对目标后再选择文件",
              "The server recorded an uncertain publication outcome. Refresh the folder and inspect the target before selecting the file again",
            ),
          );
          return;
        }
        this.unknown(
          t(
            "无法安全解读上传状态（HTTP {0}）",
            "Upload status could not be safely interpreted (HTTP {0})",
            [response.status],
          ),
          "query_upload",
        );
      } catch (error) {
        const reason = this.abortReason || errorMessage(error);
        const retryAfter = requestErrorRetryAfter(error);
        const recovery = checkpointErrorRecovery(error);
        if (queryingUnknown) {
          this.unknown(reason, recovery, retryAfter);
        } else {
          this.fail(reason, recovery, retryAfter);
        }
      } finally {
        if (this.abortController === controller) this.abortController = null;
      }
    }

    restartSession() {
      this.uploadId = crypto.randomUUID();
      this.uploadOffset = 0;
      this.uploadRequestPhase = "fresh";
      this.missingTargetRetryUsed = false;
    }

    onProgress(event: ProgressEvent) {
      if (this.phase !== "transferring") return;
      const now = Date.now();
      this.lastProgressAt = now;
      const elapsed = now - this.lastUpdate;
      if (elapsed < 300) return;
      const speed = ((event.loaded - this.uploaded) / elapsed) * 1000;
      const [speedValue, speedUnit] = formatFileSize(speed);
      const percent =
        this.file.size === 0
          ? 100
          : ((event.loaded + this.uploadOffset) / this.file.size) * 100;
      const progress = formatPercent(percent);
      const duration =
        speed > 0
          ? formatDuration((event.total - event.loaded) / speed)
          : "--:--:--";
      const announcement = Math.min(100, Math.floor(percent / 10) * 10);
      const announceNow = announcement > this.lastAnnouncedProgress;
      if (announceNow) this.lastAnnouncedProgress = announcement;
      renderProgress(
        this.view,
        this.name,
        `${speedValue} ${speedUnit}/s`,
        `${progress} ${duration}`,
        announceNow,
      );
      this.uploaded = event.loaded;
      this.lastUpdate = now;
    }

    beginCommitWait() {
      if (this.state !== "running" || this.phase !== "transferring") return;
      this.phase = "submitting";
      if (this.idleTimer !== null) window.clearTimeout(this.idleTimer);
      if (this.totalTimer !== null) window.clearTimeout(this.totalTimer);
      this.idleTimer = null;
      this.totalTimer = null;
      renderSubmitting(this.view, this.name);

      const controller = this.abortController;
      this.commitTimer = window.setTimeout(() => {
        if (
          !controller ||
          this.abortController !== controller ||
          this.phase !== "submitting"
        ) {
          return;
        }
        this.abortReason = UNKNOWN_UPLOAD_RESULT_MESSAGE;
        this.abortOutcomeUnknown = true;
        controller.abort();
      }, COMMIT_TIMEOUT_MS);
    }

    complete() {
      if (this.state !== "running") return;
      this.clearTimeouts();
      this.clearRecovery();
      this.state = "completed";
      this.phase = "completed";
      renderComplete(this.view, this.name);
      onMutation(MUTATION_EFFECT.COMMITTED);
      retainTerminalRow(this);
      this.finishRunning();
    }

    fail(
      reason: string = "",
      recovery: UploadRecoveryAction = "",
      retryAfter: number | string | null = null,
    ) {
      if (this.state !== "running") return;
      this.clearTimeouts();
      this.clearRecovery();
      this.state = "failed";
      this.phase = "failed";
      const message = reason || t("上传失败", "Upload failed");
      const recoveryButton = this.createRecoveryButton(recovery, retryAfter);
      renderFailure(this.view, this.name, message, recoveryButton);
      retainTerminalRow(
        this,
        recoveryButton
          ? () => {
              this.terminalHistoryEntry = null;
              this.clearRecovery();
            }
          : null,
      );
      this.finishRunning();
    }

    unknown(
      reason: string = UNKNOWN_UPLOAD_RESULT_MESSAGE,
      recovery: UploadRecoveryAction = "",
      retryAfter: number | string | null = null,
    ) {
      if (this.state !== "running") return;
      this.clearTimeouts();
      this.clearRecovery();
      this.state = "unknown";
      this.phase = "unknown";
      onMutation(MUTATION_EFFECT.OUTCOME_UNKNOWN);
      pauseForUnknown(this);
      const recoveryButton =
        recovery === "query_upload"
          ? this.createRecoveryButton(recovery, retryAfter)
          : null;
      renderUnknown(this.view, this.name, reason, recoveryButton);
      retainTerminalRow(
        this,
        recoveryButton
          ? () => {
              this.terminalHistoryEntry = null;
              this.clearRecovery();
            }
          : null,
      );
      this.finishRunning();
    }

    createRecoveryButton(
      recovery: UploadRecoveryAction,
      retryAfter: number | string | null,
    ): HTMLButtonElement | null {
      if (!recovery) return null;
      const label = UPLOAD_RECOVERY_LABELS[recovery];
      this.recoveryAction = recovery;
      const recoveryButton = createElement("button", {
        className: "retry-btn",
        text: "↻",
        attributes: {
          type: "button",
          title: label,
          "aria-label": `${label} ${this.name}`,
          "data-recovery": recovery,
        },
      }) as HTMLButtonElement;
      recoveryButton.addEventListener("click", () => this.recover(recovery));
      failed.set(this.index, this);

      const delaySeconds = normalizeRetryAfter(retryAfter);
      if (delaySeconds === null || delaySeconds === 0) return recoveryButton;
      recoveryButton.setAttribute("disabled", "");
      recoveryButton.title = t("{1} 秒后{0}", "{0} after {1} seconds", [
        label,
        delaySeconds,
      ]);
      const delayMs = delaySeconds * 1000;
      if (delayMs > MAX_TIMER_DELAY_MS) {
        this.recoveryAvailableAt = Number.POSITIVE_INFINITY;
        return recoveryButton;
      }
      this.recoveryAvailableAt = Date.now() + delayMs;
      this.recoveryTimer = window.setTimeout(() => {
        if (this.recoveryAction !== recovery) return;
        this.recoveryAvailableAt = 0;
        recoveryButton.removeAttribute("disabled");
        recoveryButton.title = label;
      }, delayMs);
      return recoveryButton;
    }

    clearRecovery() {
      if (this.recoveryTimer !== null) window.clearTimeout(this.recoveryTimer);
      this.recoveryTimer = null;
      this.recoveryAction = "";
      this.recoveryAvailableAt = 0;
      failed.delete(this.index);
    }

    cancel() {
      if (this.state === "queued" && queue.cancel(this.queueEntry)) {
        this.queueEntry = null;
        this.clearRecovery();
        this.state = "cancelled";
        this.phase = "cancelled";
        knownTargets.delete(this.name);
        renderCancelled(this.view, this.name);
        retainTerminalRow(this);
        runQueue();
        return;
      }
      if (this.state !== "running" || !this.abortController) return;
      if (this.requestDispatched) {
        this.abortReason = t(
          "已请求取消上传，但服务器结果不确定。{0}",
          "Upload cancellation was requested, but the server result is unknown. {0}",
          [RESULT_UNKNOWN_MESSAGE],
        );
        this.abortOutcomeUnknown = true;
      } else {
        this.abortReason = t("上传已取消", "Upload cancelled");
        this.abortOutcomeUnknown = false;
      }
      this.abortController.abort();
    }

    startTimeouts(abortRequest: EventListener) {
      this.clearTimeouts();
      const controller = new AbortController();
      controller.signal.addEventListener("abort", abortRequest, { once: true });
      this.abortController = controller;
      this.abortReason = "";
      this.abortOutcomeUnknown = false;
      this.lastProgressAt = Date.now();
      this.totalTimer = window.setTimeout(() => {
        this.abortReason = t(
          "上传超过最长允许时间",
          "Upload exceeded the maximum duration",
        );
        this.abortOutcomeUnknown = true;
        controller.abort();
      }, TOTAL_TIMEOUT_MS);
      this.scheduleIdleTimeout(controller, IDLE_TIMEOUT_MS);
    }

    scheduleIdleTimeout(controller: AbortController, delay: number) {
      if (this.idleTimer !== null) window.clearTimeout(this.idleTimer);
      this.idleTimer = window.setTimeout(() => {
        if (this.abortController !== controller) return;
        const remaining = IDLE_TIMEOUT_MS - (Date.now() - this.lastProgressAt);
        if (remaining > 0) {
          this.scheduleIdleTimeout(controller, remaining);
          return;
        }
        this.abortReason = t(
          "上传长时间没有进展",
          "Upload made no progress for too long",
        );
        this.abortOutcomeUnknown = true;
        controller.abort();
      }, delay);
    }

    clearTimeouts() {
      if (this.idleTimer !== null) window.clearTimeout(this.idleTimer);
      if (this.totalTimer !== null) window.clearTimeout(this.totalTimer);
      if (this.commitTimer !== null) window.clearTimeout(this.commitTimer);
      this.idleTimer = null;
      this.totalTimer = null;
      this.commitTimer = null;
      this.abortController = null;
      this.abortReason = "";
      this.abortOutcomeUnknown = false;
    }

    finishRunning() {
      if (!this.runningAccounted) return;
      this.runningAccounted = false;
      running = Math.max(0, running - 1);
      runQueue();
    }
  }

  async function preflight(paths: readonly string[]) {
    const { response, payload } = await requestJson(
      "/__xczs__/api/upload/preflight",
      {
        method: "POST",
        headers: {
          "Content-Type": "application/json",
          [CSRF_HEADER]: data.session.csrf_token,
        },
        body: JSON.stringify({ paths }),
      },
      {
        timeoutMs: STATUS_TIMEOUT_MS,
        timeoutMessage: t(
          "上传冲突核对超时",
          "Upload conflict check timed out",
        ),
        outcomeUnknown: false,
      },
    );
    if (!response.ok) await assertResponse(response);
    return parseUploadPreflight(payload, paths);
  }

  async function enqueueReservedSelection(
    selection: ReturnType<typeof prepareUploadSelection>,
    returnFocus: Element | null,
    reservation: PreflightReservation | null,
  ) {
    if (!selection.ok) {
      queueMessage.textContent = selection.error;
      queueMessage.classList.remove("hidden");
      return;
    }
    if (selection.entries.length === 0) return;
    if (queueState !== "running") {
      queueMessage.classList.remove("hidden");
      return;
    }

    const accepted: UploadSelectionEntry[] = [];
    const batchTargets = new Set();
    let duplicateName = "";
    for (const entry of selection.entries) {
      if (knownTargets.has(entry.name) || batchTargets.has(entry.name)) {
        duplicateName ||= entry.name;
        continue;
      }
      batchTargets.add(entry.name);
      accepted.push(entry);
    }
    if (accepted.length === 0) {
      queueMessage.textContent = duplicateName
        ? t(
            "已跳过重复上传目标 {0}。请刷新文件夹后再替换同一目标。",
            "Skipped duplicate upload target {0}. Refresh the folder before replacing the same target again.",
            [duplicateName],
          )
        : t("未选择文件。", "No files were selected.");
      queueMessage.classList.remove("hidden");
      return;
    }
    const absolutePaths = accepted.map((entry) =>
      logicalChildPath(data.href, entry.name),
    );
    if (!absolutePaths.every(isValidAbsoluteLogicalPath)) {
      queueMessage.textContent = t(
        "所选上传目标路径无效或过长。",
        "A selected upload destination is invalid or too long.",
      );
      queueMessage.classList.remove("hidden");
      return;
    }
    const pathEncoder = new TextEncoder();
    const absolutePathBytes = absolutePaths.reduce(
      (total, path) => total + pathEncoder.encode(path).byteLength,
      0,
    );
    if (absolutePathBytes > UPLOAD_BATCH_PATH_BYTES_LIMIT) {
      queueMessage.textContent = t(
        "所选上传目标超过本文件夹每批 {0} 字节的限制，请分批选择。",
        "Selected upload destinations exceed the {0}-byte batch limit in this folder. Split the selection into smaller batches.",
        [UPLOAD_BATCH_PATH_BYTES_LIMIT],
      );
      queueMessage.classList.remove("hidden");
      return;
    }
    let targets;
    try {
      targets = await preflight(absolutePaths);
    } catch (error) {
      if (isAuthenticationError(error)) {
        pauseForAuthentication();
        if (!isRequestErrorCode(error, "auth.csrf_rejected")) onUnauthorized();
        return;
      }
      queueMessage.textContent = t(
        "无法核对上传目标：{0}",
        "Unable to check upload destinations: {0}",
        [errorMessage(error)],
      );
      queueMessage.classList.remove("hidden");
      return;
    }
    if (queueState !== "running") return;

    const blockedNames = [];
    const conflicts = [];

    let uploadEntries: QueuedUploadEntry[] = [];
    for (let index = 0; index < accepted.length; index++) {
      const entry = accepted[index];
      const target = targets[index];
      if (!target.replaceable) {
        blockedNames.push(entry.name);
      } else if (target.exists) {
        conflicts.push({ entry, revision: target.revision });
      } else {
        uploadEntries.push({ ...entry, revision: null });
      }
    }

    let skippedConflicts = false;
    if (conflicts.length > 0) {
      const choice = await dialogs.chooseAction({
        title: t("已存在的上传目标", "Existing upload destinations"),
        message: t(
          "{0} {1}。仅在目标自核对后未变化时覆盖，或跳过 {2}，或取消整批。",
          "{0} {1}. Overwrite only if unchanged since this check, skip {2}, or cancel the batch.",
          [
            formatNameSummary(conflicts.map((value) => value.entry.name)),
            t(
              "已存在",
              conflicts.length === 1 ? "already exists" : "already exist",
            ),
            t("这些文件", conflicts.length === 1 ? "this file" : "these files"),
          ],
        ),
        confirmText: t("覆盖", "Overwrite"),
        alternateText: t("跳过冲突", "Skip conflicts"),
        cancelText: t("取消上传", "Cancel upload"),
        danger: true,
        returnFocus,
      });
      if (choice === "cancel" || queueState !== "running") return;
      if (choice === "confirm") {
        uploadEntries.push(
          ...conflicts.map(({ entry, revision }) => ({
            ...entry,
            revision,
          })),
        );
      } else {
        skippedConflicts = true;
      }
    }

    const selectionOrder = new Map(
      accepted.map((entry, index) => [entry.name, index]),
    );
    uploadEntries.sort(
      (left, right) =>
        (selectionOrder.get(left.name) ?? 0) -
        (selectionOrder.get(right.name) ?? 0),
    );

    // The preflight request and overwrite dialog both yield to unrelated UI
    // actions. Re-check immediately before reserving the batch so a recovery
    // that claimed the same logical target cannot be admitted concurrently.
    uploadEntries = uploadEntries.filter((entry) => {
      if (!knownTargets.has(entry.name)) return true;
      duplicateName ||= entry.name;
      return false;
    });

    if (uploadEntries.length === 0) {
      queueMessage.textContent = duplicateName
        ? t(
            "已跳过重复上传目标 {0}。请刷新文件夹后再替换同一目标。",
            "Skipped duplicate upload target {0}. Refresh the folder before replacing the same target again.",
            [duplicateName],
          )
        : blockedNames.length > 0
          ? t(
              "已跳过 {0}，因为目标无法替换。",
              "Skipped {0} because the destination cannot be replaced.",
              [formatNameSummary(blockedNames)],
            )
          : t(
              "已跳过所有冲突的上传目标。",
              "All conflicting upload destinations were skipped.",
            );
      queueMessage.classList.remove("hidden");
      return;
    }
    if (!transferPreflightRows(reservation, uploadEntries.length)) {
      showPendingLimitMessage();
      return;
    }

    // Convert the selection reservation into accounting for every admitted
    // row before yielding between DOM chunks. Recovery clicks and later
    // selections therefore cannot race the global cap.
    const batchId = nextBatchId++;
    const batch = {
      members: new Set<Uploader>(),
      enqueueComplete: false,
      cancelRequested: false,
      remainingEntries: uploadEntries.length,
    };
    batches.set(batchId, batch);

    const notices = [];
    if (duplicateName) {
      notices.push(
        t("已跳过重复上传目标 {0}。", "Skipped duplicate upload target {0}.", [
          duplicateName,
        ]),
      );
    }
    if (blockedNames.length > 0) {
      notices.push(
        t(
          "已跳过 {0}，因为目标无法替换。",
          "Skipped {0} because the destination cannot be replaced.",
          [formatNameSummary(blockedNames)],
        ),
      );
    }
    if (skippedConflicts) {
      notices.push(
        t(
          "已跳过冲突的上传目标。",
          "Skipped the conflicting upload destinations.",
        ),
      );
    }
    if (notices.length > 0) {
      queueMessage.textContent = notices.join(" ");
      queueMessage.classList.remove("hidden");
    } else {
      queueMessage.classList.add("hidden");
    }

    // Claim every logical target before the first chunking yield. Otherwise a
    // recovery click could claim an unmaterialized tail entry. Reservations
    // for entries cancelled before construction are released in `finally`.
    const unmaterializedTargets = new Set(
      uploadEntries.map((entry) => entry.name),
    );
    for (const name of unmaterializedTargets) knownTargets.add(name);

    let processed = 0;
    try {
      for (const entry of uploadEntries) {
        if (batch.cancelRequested) break;
        const uploader = new Uploader(
          entry.file,
          entry.name,
          entry.revision,
          batchId,
        );
        uploader.pendingAccounted = true;
        batch.members.add(uploader);
        uploader.enqueue();
        unmaterializedTargets.delete(entry.name);
        batch.remainingEntries--;
        processed++;
        if (processed % ENQUEUE_BATCH_SIZE === 0) await yieldToBrowser();
      }
    } finally {
      for (const name of unmaterializedTargets) knownTargets.delete(name);
      pendingRows = Math.max(0, pendingRows - batch.remainingEntries);
      batch.remainingEntries = 0;
      batch.enqueueComplete = true;
      if (batch.members.size === 0) batches.delete(batchId);
    }
  }

  async function enqueueSelection(
    selection: ReturnType<typeof prepareUploadSelection>,
    returnFocus: Element | null,
    reservation: PreflightReservation | null,
  ) {
    try {
      await enqueueReservedSelection(selection, returnFocus, reservation);
    } finally {
      releasePreflightRows(reservation);
    }
  }

  return Object.freeze({
    addFiles(
      files: FileList | File[] | null | undefined,
      addOptions: { returnFocus?: Element | null } = {},
    ) {
      // Capture the live FileList before the input is reset. Validation is
      // bounded and creates no uploader state or DOM.
      const selection = prepareUploadSelection(files);
      if (!selection.ok) {
        queueMessage.textContent = selection.error;
        queueMessage.classList.remove("hidden");
        return Promise.resolve();
      }
      if (selection.entries.length === 0) return Promise.resolve();
      const reservation = reservePreflightRows(selection.entries.length);
      if (!reservation) {
        showPendingLimitMessage();
        return Promise.resolve();
      }
      const enqueue = () =>
        enqueueSelection(
          selection,
          addOptions.returnFocus || document.activeElement,
          reservation,
        );
      enqueueTail = enqueueTail.then(enqueue, enqueue);
      return enqueueTail;
    },
    isBusy() {
      return preflightReservedRows > 0 || running > 0 || queue.size > 0;
    },
  });
}

/** Accept a target transition only for a canonical, outcome-known response
 * bound to the current upload and complete staged length. An existing target
 * requires a revision; a missing target requires that header to be absent.
 */
function trustedUploadTargetChange(
  classification: {
    kind: string;
    outcomeUnknown: boolean;
    protocol: {
      state: string;
      length: number | null;
      offset: number | null;
    } | null;
  },
  detail: { code: string; status: number },
  headers: Headers | ((name: string) => string | null),
  expectedLength: number,
  expectedResumeOffset: number | null,
):
  | { kind: "exists"; revision: string; replaceable: boolean }
  | { kind: "missing" }
  | { kind: "reset-stage" }
  | null {
  // A bound target-conflict response to PATCH is a pre-transfer rejection,
  // even though the generic running classification is conservative. Retain
  // the checkpoint and explicitly confirm the new target before resuming.
  const runningConflict = classification.kind === "running" &&
    expectedResumeOffset !== null &&
    classification.protocol?.offset === expectedResumeOffset;
  if (
    (classification.outcomeUnknown && !runningConflict) ||
    detail.status !== 409 ||
    (!["not-started", "awaiting-confirmation"].includes(classification.kind) && !runningConflict) ||
    classification.protocol?.state !== classification.kind ||
    classification.protocol.length !== expectedLength ||
    (classification.kind === "awaiting-confirmation" &&
      classification.protocol.offset !== expectedLength)
  ) {
    return null;
  }
  if (
    detail.code === "upload_metadata_preservation_refused" &&
    classification.kind === "awaiting-confirmation"
  ) {
    return { kind: "reset-stage" };
  }
  const replaceable = parseTargetReplaceable(headers);
  if (detail.code === "destination_exists") {
    const revision = parseTargetRevision(headers);
    return revision === null || replaceable === null
      ? null
      : { kind: "exists", revision, replaceable };
  }
  if (
    detail.code === "upload_target_changed" &&
    replaceable === true &&
    readHeaderValue(headers, TARGET_REVISION_HEADER) === null
  ) {
    return { kind: "missing" };
  }
  return null;
}

function readHeaderValue(
  headers: Headers | ((name: string) => string | null),
  name: string,
) {
  return typeof headers === "function" ? headers(name) : headers.get(name);
}

function formatNameSummary(names: readonly string[]) {
  const visible = names.slice(0, 5).map((name) => `"${name}"`);
  const remaining = names.length - visible.length;
  return remaining > 0
    ? t("{0}及另外 {1} 项", "{0} and {1} more", [visible.join(", "), remaining])
    : visible.join(", ");
}

function uploadFailureMessage(state: string): string {
  switch (state) {
    case "running":
      return t("上传仍可续传", "Upload remains resumable");
    case "rejected":
      return t("上传会话已被拒绝", "The upload session was rejected");
    case "not-seen":
      return t("上传未被记录", "The upload was not recorded");
    case "not-started":
      return t("上传未开始", "The upload was not started");
    default:
      return "";
  }
}

function uploadRecoveryAction(
  recovery: unknown,
  state: string,
  protocol: { state: string; offset: number | null } | null,
): UploadRecoveryAction {
  if (recovery === "query_upload") return recovery;
  if (!protocol || protocol.state !== state) return "";
  if (recovery === "retry" && state === "not-started") return "retry";
  if (
    recovery === "retry_with_new_id" &&
    ["rejected", "not-seen"].includes(state)
  ) {
    return "retry";
  }
  if (
    recovery === "resume_upload" &&
    state === "running" &&
    Number.isSafeInteger(protocol.offset)
  ) {
    return "retry";
  }
  return "";
}

function responseRetryAfter(
  request: XMLHttpRequest,
  problemRetryAfter: unknown,
): number | null {
  const rawHeader = request.getResponseHeader("Retry-After");
  return rawHeader === null
    ? normalizeRetryAfter(problemRetryAfter)
    : normalizeRetryAfter(rawHeader);
}

function requestErrorRetryAfter(error: unknown): number | null {
  const retryAfter =
    error && typeof error === "object"
      ? (error as Record<string, unknown>).retryAfter
      : null;
  return normalizeRetryAfter(retryAfter);
}

function checkpointErrorRecovery(error: unknown): UploadRecoveryAction {
  if (!error || typeof error !== "object") return "query_upload";
  const detail = error as Record<string, unknown>;
  if (detail.recovery) {
    return detail.recovery === "query_upload" ? "query_upload" : "";
  }
  return detail.kind === "http" || detail.status ? "" : "query_upload";
}

function normalizeRetryAfter(value: unknown): number | null {
  if (typeof value === "string") {
    if (!/^(0|[1-9][0-9]*)$/.test(value)) return null;
    value = Number(value);
  }
  return typeof value === "number" && Number.isSafeInteger(value) && value >= 0
    ? value
    : null;
}

function normalizeConcurrency(value: unknown): number {
  return typeof value === "number" &&
    Number.isSafeInteger(value) &&
    value > 0 &&
    value <= MAX_CLIENT_UPLOAD_CONCURRENCY
    ? value
    : DEFAULT_MAX_CONCURRENT_UPLOADS;
}

function normalizeTerminalRowLimit(value: unknown) {
  if (value === undefined) return UPLOAD_TERMINAL_ROW_LIMIT;
  if (typeof value !== "number" || !Number.isSafeInteger(value) || value <= 0) {
    throw new TypeError(
      t(
        "上传结果行数限制必须为正整数",
        "Upload terminal row limit must be a positive integer",
      ),
    );
  }
  return value;
}

function adjacentUploadNameLink(
  row: HTMLTableRowElement,
): HTMLAnchorElement | null {
  for (const sibling of [row.nextElementSibling, row.previousElementSibling]) {
    if (!(sibling instanceof HTMLTableRowElement)) continue;
    const link = sibling.querySelector(".cell-name a");
    if (link instanceof HTMLAnchorElement) return link;
  }
  return null;
}

function yieldToBrowser() {
  return new Promise((resolve) => {
    // Animation frames stop in hidden tabs and would strand the remainder of
    // a folder batch. A task still yields between DOM chunks without requiring
    // the user to keep this page visible.
    window.setTimeout(() => resolve(undefined), 0);
  });
}

function formatDuration(seconds: number) {
  if (!Number.isFinite(seconds) || seconds < 0) return "--:--:--";
  seconds = Math.ceil(seconds);
  const hours = Math.floor(seconds / 3600);
  const minutes = Math.floor((seconds - hours * 3600) / 60);
  const remaining = seconds - hours * 3600 - minutes * 60;
  return [hours, minutes, remaining]
    .map((value) => String(value).padStart(2, "0"))
    .join(":");
}

function formatPercent(percent: number) {
  return `${percent > 10 ? percent.toFixed(1) : percent.toFixed(2)}%`;
}
