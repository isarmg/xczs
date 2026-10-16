export type UploadView = {
  row: HTMLTableRowElement;
  statusCell: HTMLTableCellElement;
  speedNode: HTMLSpanElement;
  progressNode: HTMLSpanElement;
  liveNode: HTMLSpanElement;
  cancelButton: HTMLButtonElement;
};

import { t } from "../../platform.ts";
import { createElement, createIcon } from "../shared/dom.ts";

export function createUploadView(
  index: number,
  name: string,
  url: string,
  onCancel: () => void,
): UploadView {
  const row = createElement("tr", {
    className: "uploader",
    attributes: { id: `upload${index}` },
  }) as HTMLTableRowElement;
  const iconCell = createElement("td", { className: "path cell-icon" });
  iconCell.append(createIcon("file"));
  const nameCell = createElement("td", { className: "path cell-name" });
  nameCell.append(
    createElement("a", {
      text: name,
      attributes: { href: url },
    }),
  );
  const statusCell = createElement("td", {
    className: "cell-status upload-status",
    attributes: {
      id: `uploadStatus${index}`,
      "aria-label": t("{0}：等待上传", "{0}: waiting to upload", [name]),
    },
  }) as HTMLTableCellElement;
  const speedNode = createElement("span", {
    className: "upload-speed",
    attributes: { "aria-hidden": "true" },
  }) as HTMLSpanElement;
  const progressNode = createElement("span", {
    className: "upload-progress",
    attributes: { "aria-hidden": "true" },
  }) as HTMLSpanElement;
  const liveNode = createElement("span", {
    className: "visually-hidden",
    text: t("{0}：等待上传", "{0}: waiting to upload", [name]),
    attributes: { role: "status", "aria-live": "polite" },
  }) as HTMLSpanElement;
  const cancelButton = createElement("button", {
    className: "upload-cancel",
    text: t("取消", "Cancel"),
    attributes: {
      type: "button",
      "aria-label": t("取消上传 {0}", "Cancel upload {0}", [name]),
    },
  }) as HTMLButtonElement;
  cancelButton.addEventListener("click", onCancel);
  row.append(iconCell, nameCell, statusCell);
  const view = Object.freeze({
    row,
    statusCell,
    speedNode,
    progressNode,
    liveNode,
    cancelButton,
  });
  renderWaiting(view, name);
  return view;
}

export function renderWaiting(
  view: UploadView,
  name: string,
  retry: boolean = false,
) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  setCancelMode(
    view,
    t("取消", "Cancel"),
    t("取消{0} {1}", "Cancel {0} {1}", [
      retry ? t("排队重试", "queued retry") : t("排队上传", "queued upload"),
      name,
    ]),
  );
  const message = retry
    ? t("等待重试", "Waiting to retry")
    : t("等待中", "Waiting");
  view.statusCell.replaceChildren(
    createElement("span", { text: message }),
    view.cancelButton,
    view.liveNode,
  );
  announce(view, `${name}: ${message.toLowerCase()}`);
  restoreReplacedStatusFocus(view, restoreFocus, view.cancelButton);
}

export function renderProgress(
  view: UploadView,
  name: string,
  speedText: string,
  progressText: string,
  announceNow: boolean,
) {
  setCancelMode(
    view,
    t("取消", "Cancel"),
    t("取消上传 {0}", "Cancel upload {0}", [name]),
  );
  if (
    !view.statusCell.contains(view.speedNode) ||
    !view.statusCell.contains(view.progressNode) ||
    !view.statusCell.contains(view.cancelButton) ||
    !view.statusCell.contains(view.liveNode)
  ) {
    const restoreFocus = view.statusCell.contains(document.activeElement);
    // A retry after an overwrite decision re-enters progress rendering from a
    // view that intentionally has no cancel button. Rebuild the owned status
    // nodes atomically instead of using a possibly detached node as the
    // insertBefore reference.
    view.statusCell.replaceChildren(
      view.speedNode,
      view.progressNode,
      view.cancelButton,
      view.liveNode,
    );
    restoreReplacedStatusFocus(view, restoreFocus, view.cancelButton);
  }
  view.speedNode.textContent = speedText;
  view.progressNode.textContent = progressText;
  if (announceNow) {
    announce(
      view,
      t("{0}：已上传 {1}，速度 {2}", "{0}: {1} uploaded at {2}", [
        name,
        progressText,
        speedText,
      ]),
    );
  }
}

export function renderCheckpoint(view: UploadView, name: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  setCancelMode(
    view,
    t("取消", "Cancel"),
    t("取消 {0} 的续传状态核对", "Cancel resume status check for {0}", [name]),
  );
  view.statusCell.replaceChildren(
    createElement("span", {
      text: t("正在核对续传状态…", "Checking resume status…"),
    }),
    view.cancelButton,
    view.liveNode,
  );
  announce(
    view,
    t("{0}：正在核对续传状态", "{0}: checking resume status", [name]),
  );
  restoreReplacedStatusFocus(view, restoreFocus, view.cancelButton);
}

export function renderCleanup(view: UploadView, name: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", {
      text: t("正在清理暂存上传…", "Cleaning up staged upload…"),
    }),
    view.liveNode,
  );
  announce(
    view,
    t("{0}：正在清理暂存上传", "{0}: cleaning up staged upload", [name]),
  );
  restoreReplacedStatusFocus(view, restoreFocus);
}

export function renderSubmitting(view: UploadView, name: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  setCancelMode(
    view,
    t("停止等待", "Stop waiting"),
    t("停止等待上传 {0}", "Stop waiting for upload {0}", [name]),
  );
  view.statusCell.replaceChildren(
    createElement("span", { text: t("正在提交…", "Submitting…") }),
    view.cancelButton,
    view.liveNode,
  );
  announce(
    view,
    t(
      "{0}：上传数据已发送，等待服务器确认",
      "{0}: upload data sent; waiting for server confirmation",
      [name],
    ),
  );
  restoreReplacedStatusFocus(view, restoreFocus, view.cancelButton);
}

export function renderWaitingForOverwrite(view: UploadView, name: string) {
  view.statusCell.replaceChildren(
    createElement("span", {
      text: t("等待覆盖决定…", "Waiting for overwrite decision…"),
    }),
    view.liveNode,
  );
  announce(
    view,
    t("{0}：等待覆盖决定", "{0}: waiting for overwrite decision", [name]),
  );
}

export function renderComplete(view: UploadView, name: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", {
      text: "✓",
      attributes: { "aria-hidden": "true" },
    }),
    view.liveNode,
  );
  announce(view, t("{0}：上传完成", "{0}: upload complete", [name]));
  restoreReplacedStatusFocus(view, restoreFocus);
}

export function renderFailure(
  view: UploadView,
  name: string,
  reason: string,
  retryButton: HTMLButtonElement | null = null,
) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", {
      className: "upload-failure",
      text: `✗ ${reason}`,
      attributes: { title: reason },
    }),
  );
  if (retryButton) view.statusCell.append(retryButton);
  view.statusCell.append(view.liveNode);
  announce(view, `${name}: ${reason}`);
  restoreReplacedStatusFocus(view, restoreFocus, retryButton);
}

export function renderUnknown(
  view: UploadView,
  name: string,
  reason: string,
  recoveryButton: HTMLButtonElement | null = null,
) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", {
      className: "upload-failure upload-unknown",
      text: `? ${reason}`,
      attributes: { title: reason },
    }),
  );
  if (recoveryButton) view.statusCell.append(recoveryButton);
  view.statusCell.append(view.liveNode);
  announce(
    view,
    t("{0}：上传结果不确定；{1}", "{0}: upload result unknown; {1}", [
      name,
      reason,
    ]),
  );
  restoreReplacedStatusFocus(view, restoreFocus, recoveryButton);
}

export function renderCancelled(view: UploadView, name: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", { text: t("已取消", "Cancelled") }),
    view.liveNode,
  );
  announce(view, t("{0}：上传已取消", "{0}: upload cancelled", [name]));
  restoreReplacedStatusFocus(view, restoreFocus);
}

export function renderSkipped(view: UploadView, name: string, reason: string) {
  const restoreFocus = view.statusCell.contains(document.activeElement);
  view.statusCell.replaceChildren(
    createElement("span", {
      text: t("已跳过（{0}）", "Skipped ({0})", [reason]),
    }),
    view.liveNode,
  );
  announce(
    view,
    t("{0}：已跳过，原因：{1}", "{0}: skipped because the {1}", [name, reason]),
  );
  restoreReplacedStatusFocus(view, restoreFocus);
}

/** Keep keyboard focus in a useful place when a status transition removes the
 * currently focused Cancel/Stop-waiting control.
 */
function restoreReplacedStatusFocus(
  view: UploadView,
  restoreFocus: boolean,
  preferredFocus: HTMLButtonElement | null = null,
) {
  if (!restoreFocus) return;

  let target: HTMLElement | null = preferredFocus;
  if (!target?.isConnected || target.hasAttribute("disabled")) {
    const nameLink = view.row.querySelector(".cell-name a");
    target = nameLink instanceof HTMLElement ? nameLink : null;
  }
  if (target) {
    target.focus({ preventScroll: true });
    return;
  }
  view.statusCell.tabIndex = -1;
  view.statusCell.focus({ preventScroll: true });
}

function announce(view: UploadView, message: string) {
  view.statusCell.setAttribute("aria-label", message);
  view.liveNode.textContent = message;
}

function setCancelMode(view: UploadView, text: string, accessibleName: string) {
  view.cancelButton.textContent = text;
  view.cancelButton.setAttribute("aria-label", accessibleName);
}
