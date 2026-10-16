export type ActionDialogs = {
  showMessage: (options: DialogOptions) => Promise<undefined>;
  confirmAction: (options: DialogOptions) => Promise<boolean>;
  chooseAction: (options: DialogOptions) => Promise<DialogChoice>;
  requestText: (options: DialogOptions) => Promise<string | null>;
};

export type DialogChoice = "confirm" | "alternate" | "cancel";

export type DialogOptions = {
  title: string;
  message?: string;
  label?: string;
  value?: string;
  confirmText?: string;
  alternateText?: string;
  cancelText?: string;
  danger?: boolean;
  returnFocus?: Element | null;
};

import { t } from "../../platform.ts";

export function createActionDialogs(): ActionDialogs {
  const dialog = requiredDialogElement(
    ".action-dialog",
    document,
    HTMLDialogElement,
  );
  const title = requiredDialogElement(
    "#action-dialog-title",
    dialog,
    HTMLElement,
  );
  const message = requiredDialogElement(
    "#action-dialog-message",
    dialog,
    HTMLElement,
  );
  const inputGroup = requiredDialogElement(
    ".action-dialog-input-group",
    dialog,
    HTMLElement,
  );
  const inputLabel = requiredDialogElement("label", inputGroup, HTMLElement);
  const input = requiredDialogElement(
    "#action-dialog-input",
    inputGroup,
    HTMLInputElement,
  );
  const cancelButton = requiredDialogElement(
    ".action-dialog-cancel",
    dialog,
    HTMLButtonElement,
  );
  const alternateButton = requiredDialogElement(
    ".action-dialog-alternate",
    dialog,
    HTMLButtonElement,
  );
  const confirmButton = requiredDialogElement(
    ".action-dialog-confirm",
    dialog,
    HTMLButtonElement,
  );

  let active: {
    kind: "alert" | "confirm" | "choice" | "prompt";
    resolve: (value: unknown) => void;
    returnFocus: Element | null;
  } | null = null;

  let queue: Promise<void> = Promise.resolve();

  cancelButton.addEventListener("click", () => dialog.close("cancel"));
  dialog.addEventListener("cancel", (event) => {
    event.preventDefault();
    dialog.close("cancel");
  });
  dialog.addEventListener("keydown", (event) => {
    if (event.key !== "Tab" || !active) return;
    const controls = [
      ...(input.disabled ? [] : [input]),
      ...(cancelButton.hidden ? [] : [cancelButton]),
      ...(alternateButton.hidden ? [] : [alternateButton]),
      confirmButton,
    ];
    const currentIndex = controls.findIndex(
      (control) => control === document.activeElement,
    );
    const nextControl = event.shiftKey
      ? currentIndex <= 0
        ? controls.at(-1)
        : null
      : currentIndex === -1 || currentIndex === controls.length - 1
        ? controls[0]
        : null;
    if (!nextControl) return;
    event.preventDefault();
    nextControl.focus();
  });
  dialog.addEventListener("close", () => {
    if (!active) return;
    const request = active;
    active = null;
    const confirmed = dialog.returnValue === "confirm";
    let result;
    if (request.kind === "prompt") {
      result = confirmed ? input.value : null;
    } else if (request.kind === "confirm") {
      result = confirmed;
    } else if (request.kind === "choice") {
      result = dialogChoice(dialog.returnValue);
    }
    restoreFocus(request.returnFocus);
    request.resolve(result);
  });

  function enqueue(
    options: DialogOptions & {
      kind: "alert" | "confirm" | "choice" | "prompt";
    },
  ) {
    const result = queue.then(() => show(options));
    queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  }

  function show(
    options: DialogOptions & {
      kind: "alert" | "confirm" | "choice" | "prompt";
    },
  ) {
    const kind = options.kind;
    title.textContent = options.title;
    message.textContent = options.message || "";
    message.hidden = !options.message;
    if (options.message) {
      dialog.setAttribute("aria-describedby", "action-dialog-message");
    } else {
      dialog.removeAttribute("aria-describedby");
    }
    inputGroup.hidden = kind !== "prompt";
    input.disabled = kind !== "prompt";
    input.required = kind === "prompt";
    input.value = options.value || "";
    inputLabel.textContent = options.label || "";
    cancelButton.hidden = kind === "alert";
    cancelButton.textContent = options.cancelText || t("取消", "Cancel");
    alternateButton.hidden = kind !== "choice";
    // Hidden submit buttons still participate in native implicit submission.
    // A text prompt must submit Confirm when Enter is pressed in its input.
    alternateButton.type = kind === "choice" ? "submit" : "button";
    alternateButton.textContent = options.alternateText || t("跳过", "Skip");
    confirmButton.textContent =
      options.confirmText ||
      (kind === "alert" ? t("关闭", "Close") : t("确认", "Confirm"));
    confirmButton.classList.toggle("danger", Boolean(options.danger));
    dialog.returnValue = "";

    return new Promise((resolve) => {
      active = {
        kind,
        resolve,
        returnFocus: options.returnFocus || document.activeElement,
      };
      dialog.showModal();
      (kind === "prompt" ? input : confirmButton).focus();
      if (kind === "prompt") input.select();
    });
  }

  return Object.freeze({
    showMessage(options: DialogOptions) {
      return enqueue({ ...options, kind: "alert" }).then(() => undefined);
    },

    confirmAction(options: DialogOptions) {
      return enqueue({ ...options, kind: "confirm" }).then(
        (value) => value === true,
      );
    },

    chooseAction(options: DialogOptions): Promise<DialogChoice> {
      return enqueue({ ...options, kind: "choice" }).then(dialogChoice);
    },

    requestText(options: DialogOptions) {
      return enqueue({ ...options, kind: "prompt" }).then((value) =>
        typeof value === "string" ? value : null,
      );
    },
  });
}

function dialogChoice(value: unknown): DialogChoice {
  return value === "confirm" || value === "alternate" ? value : "cancel";
}

function requiredDialogElement<T extends Element>(
  selector: string,
  root: ParentNode,
  constructor: { new (...args: never[]): T },
): T {
  const element = root.querySelector(selector);
  if (!element)
    throw new Error(
      t(
        "缺少必要的对话框控件：{0}",
        "Required dialog control is missing: {0}",
        [selector],
      ),
    );
  if (!(element instanceof constructor)) {
    throw new Error(
      t(
        "必要的对话框控件类型不正确：{0}",
        "Required dialog control has the wrong type: {0}",
        [selector],
      ),
    );
  }
  return element;
}

function restoreFocus(element: Element | null | undefined) {
  if (!(element instanceof HTMLElement) || !element.isConnected) return;
  element.focus({ preventScroll: true });
}
