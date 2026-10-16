import {
  createElement as h,
  Fragment,
  useEffect,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createRoot } from "react-dom/client";
import { flushSync } from "react-dom";
import {
  Button,
  IconButton,
  PageHeader,
  TextField,
  Table,
} from "@xcss/web/admin-ui";
import {
  AccountSettings,
  AccountPage,
  LoginControls,
  ThemeToggle,
  WorkspaceIcon,
  resolveWorkspaceConfig,
} from "@xcss/web/admin-shell";
import {
  t,
  initializeLanguage,
  languageLabel,
  switchLanguage,
  validationMessage,
} from "@xcss/web/admin-ui/i18n";
import { isAdministratorLoginRequest } from "@xcss/web/contracts";
import { isAdministratorPassword } from "@xcss/web/admin-web";
import {
  navigationEntries,
  workspaceView,
  type WorkspaceView,
} from "./navigation.ts";
import { FileLibrary } from "./tags.tsx";
import {
  FileSearch,
  FileTagsToolbar,
} from "./directory-tags.tsx";
import {
  fileActionState,
  subscribeFileAction,
  selectFileAction,
  toggleFileAction,
  type FileAction,
} from "../modules/listing/action-mode.ts";

const mounted: WeakSet<Element> = new WeakSet();
const workspace = resolveWorkspaceConfig();

/** A single React root owns the authored page. File controllers own only the
 * opaque file/queue/dialog regions after the initial synchronous render.
 * Theme updates are confined to their own component, so React never reconciles
 * an active upload row or inline editor. Menu navigation retains file controllers.
 */
function mount(container: HTMLElement, page: import("react").ReactNode) {
  if (mounted.has(container))
    throw new Error("Xczs application is already mounted");
  if (document.documentElement.dataset.xcssFonts !== "ready") {
    throw new Error("Xczs application fonts must be prepared before mounting");
  }
  mounted.add(container);
  initializeLanguage();
  document.documentElement.dataset.xcssAppearance = workspace.appearance;
  document.documentElement.dataset.xcssSelection = workspace.selection;
  document.documentElement.style.setProperty(
    "--xcss-font-ui",
    workspace.fontFamily,
  );
  document.documentElement.style.setProperty(
    "--xcss-header-icon-size",
    workspace.headerIconSize,
  );
  const root = createRoot(container);
  flushSync(() => root.render(page));
}

export function mountFileWorkspace(
  container: HTMLElement,
  client: import("@xcss/web/admin-web").AdministratorApiClient,
  directory: string,
) {
  mount(container, h(FileWorkspace, { client, directory }));
}

export function mountLoginPage(
  container: HTMLElement,
  login: (username: string, password: string) => Promise<void>,
  errorMessage: (error: unknown) => string,
) {
  const minimum = Number(container.dataset.minBytes);
  const maximum = Number(container.dataset.maxBytes);
  if (
    !Number.isInteger(minimum) ||
    !Number.isInteger(maximum) ||
    minimum < 1 ||
    maximum < minimum
  ) {
    throw new Error("Invalid administrator password contract");
  }
  mount(container, h(LoginPage, { login, errorMessage, minimum, maximum }));
  document.title = t("登录", "Sign in");
}

function LanguageToggle() {
  return h(
    IconButton,
    {
      "aria-label": languageLabel(),
      title: languageLabel(),
      onClick: switchLanguage,
    },
    h(
      "svg",
      {
        "aria-hidden": true,
        width: "1em",
        height: "1em",
        viewBox: "0 0 24 24",
        fill: "none",
        stroke: "currentColor",
        strokeWidth: 1.7,
      },
      h("path", {
        d: "M3 5h12M9 3v2M6 5c0 5 4 9 8 11M13 5c0 5-4 9-9 12m10 4 4-10 4 10m-6.5-4h5",
      }),
    ),
  );
}

function AccountEntry({
  client,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
}) {
  const [session, setSession] = useState(() => client.currentSession());
  useEffect(() => client.subscribe(setSession), [client]);
  return session ? h(AccountSettings) : null;
}

function Header({
  client,
  view,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
  view: WorkspaceView;
}) {
  return h(
    PageHeader,
    null,
    h(
      "div",
      { className: "xcss-header-navigation-slot" },
      h(
        "div",
        { className: "xcss-header-brand-navigation" },
        h("strong", { className: "xcss-product-identity" }, "xczs"),
        h(
          "nav",
          {
            className: "xcss-header-navigation",
            "aria-label": t("主导航", "Main navigation"),
          },
          navigationEntries().map((item) =>
            h(
              "a",
              {
                key: item.id,
                href: item.href,
                "aria-current": item.id === view ? "page" : undefined,
              },
              item.label,
            ),
          ),
        ),
      ),
    ),
    h(
      "div",
      {
        className: "toolbox-right xcss-header-actions",
        role: "group",
        "aria-label": t("全局操作", "Global actions"),
      },
      h(
        IconButton,
        {
          "aria-label": t("重新载入页面", "Reload page"),
          title: t("重新载入页面", "Reload page"),
          onClick: () => window.location.reload(),
        },
        h(WorkspaceIcon, { name: "refresh" }),
      ),
      h(LanguageToggle),
      h(ThemeToggle),
      h(
        IconButton,
        {
          className: "logout-btn hidden",
          "aria-label": t("退出", "Sign out"),
          title: t("退出", "Sign out"),
        },
        h(WorkspaceIcon, { name: "logout" }),
        h("span", { className: "user-name", hidden: true }),
      ),
      h(AccountEntry, { client }),
    ),
  );
}

function FileActions({
  client,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
}) {
  const { action } = useSyncExternalStore(subscribeFileAction, fileActionState);
  const actions: readonly [FileAction, string][] = [
    ["move", t("移动", "Move")],
    ["download", t("下载", "Download")],
    ["delete", t("删除", "Delete")],
    ["rename", t("重命名", "Rename")],
    ["tags", t("标签", "Tags")],
  ];
  return h(
    "div",
    {
      className: "xczs-file-actions xcss-secondary-navigation",
      role: "group",
      "aria-label": t("文件操作", "File controls"),
    },
    h("nav", {
      className: "xczs-root-navigation",
      "aria-label": t("根目录导航", "Root directory navigation"),
    }),
    h(
      IconButton,
      {
        className: "control upload-file hidden",
        "aria-label": t("上传文件", "Upload files"),
        title: t("上传文件", "Upload files"),
      },
      h(FileIcon, { name: "upload" }),
    ),
    h("input", {
      className: "visually-hidden",
      type: "file",
      id: "file",
      name: "file",
      "aria-label": t("选择要上传的文件", "Choose files to upload"),
      tabIndex: -1,
      multiple: true,
    }),
    h(
      IconButton,
      {
        className: "control upload-folder hidden",
        "aria-label": t("上传文件夹", "Upload folder"),
        title: t("上传文件夹", "Upload folder"),
      },
      h(FileIcon, { name: "folder" }),
    ),
    h("input", {
      className: "visually-hidden",
      type: "file",
      id: "folder",
      name: "folder",
      "aria-label": t("选择要上传的文件夹", "Choose a folder to upload"),
      tabIndex: -1,
      webkitdirectory: "",
      multiple: true,
    }),
    h(
      IconButton,
      {
        className: "control new-folder hidden",
        "aria-label": t("新建文件夹", "New folder"),
        title: t("新建文件夹", "New folder"),
      },
      h(WorkspaceIcon, { name: "create" }),
    ),
    h(
      IconButton,
      {
        className: "control new-file hidden",
        "aria-label": t("新建空文件", "New empty file"),
        title: t("新建空文件", "New empty file"),
      },
      h(FileIcon, { name: "file" }),
    ),
    ...actions.map(([name, label]) => h(IconButton, {
      key: name,
      className: "control file-action-mode",
      ...{ "data-file-action": name },
      "aria-label": label,
      title: label,
      "aria-pressed": action === name,
      onClick: (event: import("react").MouseEvent<HTMLButtonElement>) => {
        if (event.currentTarget.getAttribute("aria-disabled") !== "true") toggleFileAction(name);
      },
    }, h(FileIcon, { name }))),
    h(FileSearch, { client }),
  );
}

function FileIcon({ name }: { name: "upload" | "folder" | "file" | "search" | FileAction }) {
  const paths = {
    upload: "M12 16V3m-5 5 5-5 5 5M4 15v6h16v-6",
    folder: "M3 7V4h6l3 3h9v14H3V7m9 11v-8m-3 3 3-3 3 3",
    file: "M5 3h9l5 5v13H5V3m9 0v6h5M9 15h6m-3-3v6",
    search: "M17 17l5 5M19 10a9 9 0 1 1-18 0 9 9 0 0 1 18 0",
    move: "M4 4v7a5 5 0 0 0 5 5h11m-5-5 5 5-5 5",
    download: "M12 3v13m-5-5 5 5 5-5M4 18v3h16v-3",
    delete: "M4 6h16M9 6V3h6v3M6 6l1 15h10l1-15M10 10v7m4-7v7",
    rename: "m4 17-1 4 4-1L21 6l-4-4L4 17m10-12 4 4",
    tags: "M3 3h8l10 10-8 8L3 11V3m4 4h.01",
  };
  return h(
    "svg",
    {
      "aria-hidden": true,
      width: 16,
      height: 16,
      viewBox: "0 0 24 24",
      fill: "none",
      stroke: "currentColor",
      strokeWidth: 1.7,
    },
    h("path", { d: paths[name] }),
  );
}

function FileActionHint() {
  const { action, notice } = useSyncExternalStore(subscribeFileAction, fileActionState);
  const hints = {
    move: t("请选择要移动的文件或文件夹。", "Choose a file or folder to move."),
    download: t("请选择要下载的文件。", "Choose a file to download."),
    delete: t("请选择要删除的文件或文件夹。", "Choose a file or folder to delete."),
    rename: t("请选择要重命名的文件或文件夹。", "Choose a file or folder to rename."),
    tags: t("请选择文件，然后点击下方标签添加或移除。", "Choose a file, then click a tag below to add or remove it."),
  };
  return h("div", { className: "file-action-hint", hidden: !action },
    h("span", { id: "file-action-hint", role: "status" }, notice || (action ? hints[action] : "")),
    h(Button, { onClick: () => selectFileAction(null) }, t("取消", "Cancel")),
  );
}

function FileControls() {
  return h(
    "section",
    {
      className: "xcss-content-panel file-toolbar",
      "aria-label": t("文件操作", "File controls"),
    },
    h("nav", {
      className: "breadcrumb",
      "aria-label": t("当前文件夹", "Current folder"),
    }),
  );
}

function FileContent() {
  return h(
    "div",
    { className: "index-page xcss-content-panel hidden" },
    h("div", {
      className: "operation-status hidden",
      role: "status",
      "aria-live": "polite",
    }),
    h("div", {
      className: "upload-queue-message hidden",
      role: "alert",
      "aria-live": "assertive",
    }),
    h("div", {
      className: "empty-folder hidden",
      role: "status",
      "aria-live": "polite",
    }),
    h(
      Table,
      {
        className: "uploaders-table hidden",
        "aria-label": t("上传队列", "Upload queue"),
      },
      h("caption", {
        className: "upload-history-status hidden",
        role: "status",
        "aria-live": "polite",
      }),
      h(
        "thead",
        null,
        h(
          "tr",
          null,
          h("th", { className: "cell-name", colSpan: 2 }, t("名称", "Name")),
          h("th", { className: "cell-status" }, t("上传状态", "Upload status")),
        ),
      ),
    ),
    h(
      Table,
      {
        className: "paths-table hidden",
        "aria-label": t("文件列表", "File list"),
      },
      h(
        "colgroup",
        null,
        h("col", { className: "file-icon" }),
        h("col"),
        h("col", { className: "file-tags" }),
        h("col", { className: "file-modified" }),
        h("col", { className: "file-size" }),
      ),
      h("thead"),
      h("tbody"),
    ),
    h(
      "div",
      { className: "list-pagination" },
      h("div", {
        className: "list-status",
        role: "status",
        "aria-live": "polite",
      }),
      h(Button, { className: "load-more hidden" }, t("加载更多", "Load more")),
    ),
  );
}

function ActionDialog() {
  return h(
    "dialog",
    {
      className: "action-dialog xcss-dialog",
      "aria-labelledby": "action-dialog-title",
      "aria-describedby": "action-dialog-message",
    },
    h(
      "form",
      { className: "action-dialog-form", method: "dialog" },
      h("h2", { id: "action-dialog-title" }),
      h("p", { id: "action-dialog-message" }),
      h(
        "div",
        { className: "action-dialog-input-group" },
        h("label", { htmlFor: "action-dialog-input" }),
        h("input", {
          id: "action-dialog-input",
          type: "text",
          autoComplete: "off",
        }),
      ),
      h(
        "div",
        { className: "action-dialog-actions" },
        h(Button, { className: "action-dialog-cancel" }, t("取消", "Cancel")),
        h(
          Button,
          {
            className: "action-dialog-alternate",
            type: "button",
            value: "alternate",
            hidden: true,
          },
          t("跳过", "Skip"),
        ),
        h(
          Button,
          {
            className: "action-dialog-confirm",
            type: "submit",
            value: "confirm",
          },
          t("确认", "Confirm"),
        ),
      ),
    ),
  );
}

function FileWorkspace({
  client,
  directory,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
  directory: string;
}) {
  const [view, setView] = useState(workspaceView);
  const [session, setSession] = useState(() => client.currentSession());
  useEffect(() => client.subscribe(setSession), [client]);
  useEffect(() => {
    const changed = () => {
      setView(workspaceView());
    };
    window.addEventListener("hashchange", changed);
    window.addEventListener("popstate", changed);
    return () => {
      window.removeEventListener("hashchange", changed);
      window.removeEventListener("popstate", changed);
    };
  }, []);
  useEffect(() => {
    selectFileAction(null);
  }, [view]);
  const libraryView = view === "tags" || view === "status" ? view : null;
  return h(
    Fragment,
    null,
    h(
      "a",
      {
        className: "xcss-skip-link",
        href: "#file-content",
        onClick: (event) => {
          event.preventDefault();
          document.getElementById("file-content")?.focus();
        },
      },
      t("跳转到文件内容", "Skip to files"),
    ),
    h(Header, { client, view }),
    h("p", { className: "workspace-status hidden", role: "alert" }),
    h(
      "main",
      {
        className: "main xcss-shell-main",
        id: "file-content",
        tabIndex: -1,
        "data-workspace-view": view,
      },
      view === "unknown" && h(
        "p",
        { className: "workspace-navigation-error", role: "alert" },
        t("页面不存在，请选择上方菜单。", "Page not found. Choose a menu above."),
      ),
      h(FileActions, { client }),
      h(FileActionHint),
      h(FileTagsToolbar, { client }),
      h(FileControls),
      h(FileContent),
      session && h(FileLibrary, { client, view: libraryView, directory }),
      view === "account" && h(AccountView, { client }),
    ),
    h(ActionDialog),
  );
}

const AccountContent: import("react").FunctionComponent<
  NonNullable<Parameters<typeof AccountPage>[0]>
> = AccountPage;

function AccountView({
  client,
}: {
  client: import("@xcss/web/admin-web").AdministratorApiClient;
}) {
  const [session, setSession] = useState(() => client.currentSession());
  useEffect(() => client.subscribe(setSession), [client]);
  return session
    ? h(AccountContent, {
        client,
        username: session.username,
        onUpdated: () => {
          window.location.href = "/__xczs__/login";
        },
      })
    : null;
}

function LoginPage({
  login,
  errorMessage,
  minimum,
  maximum,
}: {
  login(username: string, password: string): Promise<void>;
  errorMessage(error: unknown): string;
  minimum: number;
  maximum: number;
}) {
  const [pending, setPending] = useState(false);
  const busy = useRef(false);
  const [failure, setFailure] = useState({ message: "", field: "" });

  async function submit(event: import("react").FormEvent<HTMLElement>) {
    event.preventDefault();
    if (busy.current) return;
    const form = event.currentTarget;
    if (!(form instanceof HTMLFormElement)) return;
    const username = form.elements.namedItem("username");
    const password = form.elements.namedItem("password");
    if (
      !(username instanceof HTMLInputElement) ||
      !(password instanceof HTMLInputElement)
    )
      return;
    for (const input of [username, password]) {
      if (!input.validity.valid) {
        const message = input.validity.valueMissing
          ? input === username
            ? t("请输入用户名。", "Enter your username.")
            : t("请输入密码。", "Enter your password.")
          : validationMessage(input);
        setFailure({ message, field: input.name });
        input.focus();
        return;
      }
    }
    if (
      !isAdministratorLoginRequest({
        username: username.value,
        password: password.value,
      }) ||
      !isAdministratorPassword(password.value)
    ) {
      password.value = "";
      setFailure({
        message: t(
          "请输入有效的管理员用户名和密码。",
          "Enter a valid administrator username and password.",
        ),
        field: "password",
      });
      password.focus();
      return;
    }
    busy.current = true;
    setPending(true);
    setFailure({ message: "", field: "" });
    // Apply the lock immediately, before another browser event can submit.
    username.readOnly = true;
    password.readOnly = true;
    let failed = false;
    try {
      await login(username.value, password.value);
    } catch (error) {
      failed = true;
      setFailure({ message: errorMessage(error), field: "" });
    } finally {
      password.value = "";
      username.readOnly = false;
      password.readOnly = false;
      busy.current = false;
      setPending(false);
      if (failed) password.focus();
    }
  }
  return h(
    "main",
    { className: "xcss-auth-shell login-screen" },
    h(
      "div",
      {
        className: "xcss-auth-language",
        style: {
          position: "absolute",
          insetBlockStart: "1rem",
          insetInlineEnd: "1rem",
        },
      },
      h(LoginControls, null, h(LanguageToggle)),
    ),
    h(
      "section",
      { className: "xcss-auth-card", "aria-label": "xczs" },
      h(
        "form",
        {
          className: "login-card",
          "aria-label": t("登录 xczs", "Sign in to xczs"),
          noValidate: true,
          "aria-busy": pending || undefined,
          onSubmit: (event) => {
            void submit(event);
          },
          onInvalid: (event) => event.preventDefault(),
          onInput: () => setFailure({ message: "", field: "" }),
        },
        h("h1", null, t("登录", "Sign in")),
        h(
          "label",
          { className: "xcss-form-field", htmlFor: "username" },
          h("span", null, t("用户名", "Username")),
          h(TextField, {
            className: "login-input",
            id: "username",
            name: "username",
            type: "text",
            maxLength: 64,
            autoComplete: "username",
            autoCapitalize: "none",
            spellCheck: false,
            required: true,
            autoFocus: true,
            readOnly: pending,
            "aria-invalid": failure.field === "username" || undefined,
            "aria-describedby": failure.message ? "login-error" : undefined,
          }),
        ),
        h(
          "label",
          { className: "xcss-form-field", htmlFor: "password" },
          h("span", null, t("密码", "Password")),
          h(TextField, {
            className: "login-input",
            id: "password",
            name: "password",
            type: "password",
            ...{ "data-min-bytes": minimum, "data-max-bytes": maximum },
            autoComplete: "current-password",
            required: true,
            readOnly: pending,
            "aria-invalid": failure.field === "password" || undefined,
            "aria-describedby": failure.message ? "login-error" : undefined,
          }),
        ),
        h(
          "div",
          {
            className: `xcss-error error-row${failure.message ? "" : " hidden"}`,
            role: "alert",
          },
          h(
            "span",
            { className: "login-error", id: "login-error" },
            failure.message,
          ),
        ),
        h(Button, { type: "submit", disabled: pending }, t("登录", "Sign in")),
      ),
    ),
  );
}
