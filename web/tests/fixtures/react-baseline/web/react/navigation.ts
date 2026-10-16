import { t } from "@xcss/web/admin-ui/i18n";

export type WorkspaceView = "files" | "tags" | "status" | "account" | "unknown";

const sections = [
  { id: "files", zh: "文件", en: "Files", href: "#files" },
  {
    id: "tags",
    zh: "标签管理",
    en: "Manage tags",
    href: "#tags",
  },
  {
    id: "status",
    zh: "服务状态",
    en: "Service status",
    href: "#status",
  },
] as const;

export function workspaceView(hash = window.location.hash): WorkspaceView {
  if (hash === "" || hash === "#" || hash === "#files") return "files";
  if (hash === "#tags") return "tags";
  if (hash === "#status") return "status";
  if (hash === "#account") return "account";
  return "unknown";
}

export function navigationEntries() {
  return sections.map((section) => ({
    id: section.id,
    label: t(section.zh, section.en),
    href: section.href,
  }));
}
