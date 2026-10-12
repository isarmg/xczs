import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { AdministratorApiClient } from "@xcss/web/admin-web";
import {
  Button,
  ErrorState,
  FormField,
  LoadingState,
  Select,
} from "@xcss/web/admin-ui";
import { t } from "@xcss/web/admin-ui/i18n";
import {
  cursorQuery,
  tagsPage,
  scanResponse,
  type TagRow,
  type TagLabel,
  type TagPage,
  type ListedTags,
  listedTags,
} from "./tags-pages.ts";
import { tagRequest, tagFailure } from "./tag-api.ts";
import { fileActionState, subscribeFileAction } from "../modules/listing/action-mode.ts";

const filterLabels = () =>
  [
    ["all", t("全部标签", "All tags")],
    ["any", t("任一标签", "Any tag")],
    ["exclude", t("排除标签", "Exclude tags")],
  ] as const;
const valuesFromLocation = () => {
  const query = new URLSearchParams(window.location.search);
  return Object.fromEntries(
    ["all", "any", "exclude"].map((key) => [
      key,
      (query.get(key) ?? "")
        .split(",")
        .filter((id) => /^[1-9]\d{0,18}$/.test(id))
        .slice(0, 100),
    ]),
  ) as Record<"all" | "any" | "exclude", string[]>;
};

export function FileSearch({ client }: { client: AdministratorApiClient }) {
  const [open, setOpen] = useState(false);
  const [filters, setFilters] = useState(valuesFromLocation);
  const [tags, setTags] = useState<TagRow[]>([]);
  const [page, setPage] = useState<TagPage | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  useEffect(() => {
    const changed = () => setFilters(valuesFromLocation());
    window.addEventListener("popstate", changed);
    return () => window.removeEventListener("popstate", changed);
  }, []);
  useEffect(() => {
    if (!open) return;
    let live = true;
    const controller = new AbortController();
    tagRequest(
      client,
      `/tags${cursorQuery(cursor)}`,
      tagsPage,
      undefined,
      undefined,
      controller.signal,
    )
      .then((value) => {
        if (live) {
          setError("");
          setPage(value);
          setTags((previous) =>
            [
              ...new Map(
                [...previous, ...value.tags].map((tag) => [tag.id, tag]),
              ).values(),
            ].filter(
              (tag) =>
                value.tags.some((value) => value.id === tag.id) ||
                Object.values(filters).some((ids) =>
                  ids.includes(String(tag.id)),
                ),
            ),
          );
        }
      })
      .catch((cause) => {
        if (live) setError(tagFailure(cause));
      });
    return () => {
      live = false;
      controller.abort();
    };
  }, [client, open, cursor, version]);
  const count = new Set(Object.values(filters).flat()).size;
  return (
    <form
      className="searchbar hidden"
      aria-label={t("搜索文件和标签", "Search files and tags")}
      onReset={() => {
        setFilters({ all: [], any: [], exclude: [] });
      }}
    >
      <div className="search-input">
        <label className="visually-hidden" htmlFor="search">
          {t("搜索文件或文件夹", "Search files or folders")}
        </label>
        <input
          id="search"
          name="q"
          type="text"
          maxLength={128}
          autoComplete="off"
          aria-label={t("搜索文件或文件夹", "Search files or folders")}
          placeholder={t("搜索文件或文件夹", "Search files or folders")}
        />
      </div>
      <details
        className="search-tags"
        onToggle={(event) => setOpen(event.currentTarget.open)}
      >
        <summary aria-label={t("标签筛选", "Tag filters")}>
          {t("标签", "Tags")}
          {count > 0 ? ` (${count})` : ""}
        </summary>
        <div className="search-tag-options">
          {error && (
            <ErrorState
              onRetry={() => {
                setError("");
                setVersion((value) => value + 1);
              }}
            >
              {error}
            </ErrorState>
          )}
          {filterLabels().map(([key, label]) => (
            <FormField key={key} label={label}>
              <Select
                multiple
                name={key}
                size={3}
                aria-label={label}
                value={filters[key]}
                onChange={(event) => {
                  const ids = Array.from(
                    event.currentTarget.selectedOptions,
                    (option) => option.value,
                  );
                  if (ids.length <= 100)
                    setFilters((current) => ({ ...current, [key]: ids }));
                }}
              >
                {filters[key]
                  .filter((id) => !tags.some((tag) => String(tag.id) === id))
                  .map((id) => (
                    <option key={id} value={id}>
                      #{id}
                    </option>
                  ))}
                {tags.map((tag) => (
                  <option key={tag.id} value={String(tag.id)}>
                    {tag.name}
                  </option>
                ))}
              </Select>
            </FormField>
          ))}
          <div className="search-tag-pager">
            <Button
              disabled={!page?.previous_cursor}
              onClick={() => setCursor(page?.previous_cursor ?? null)}
            >
              {t("上一页", "Previous page")}
            </Button>
            <Button
              disabled={!page?.next_cursor}
              onClick={() => setCursor(page?.next_cursor ?? null)}
            >
              {t("下一页", "Next page")}
            </Button>
          </div>
          <div className="search-tag-actions">
            <Button type="submit">{t("搜索", "Search")}</Button>
            <Button
              onClick={() => {
                setFilters({ all: [], any: [], exclude: [] });
                window.dispatchEvent(new Event("xczs:clear-tag-filters"));
              }}
            >
              {t("清除标签筛选", "Clear tag filters")}
            </Button>
          </div>
        </div>
      </details>
      <Button
        type="submit"
        className="search-submit"
        aria-label={t("搜索", "Search")}
      >
        ↵
      </Button>
    </form>
  );
}

export type FileTagTarget = {
  name: string;
  path: string;
  fileId: number | null;
};
const empty = (value: unknown): value is undefined => value === undefined;

export function FileTagsToolbar({ client }: { client: AdministratorApiClient }) {
  const { action } = useSyncExternalStore(subscribeFileAction, fileActionState);
  const [target, setTarget] = useState<FileTagTarget | null>(null);
  useEffect(() => {
    const selected = (event: Event) => {
      if (event instanceof CustomEvent && fileActionState().action === "tags")
        setTarget(event.detail as FileTagTarget);
    };
    window.addEventListener("xczs:file-tags", selected);
    return () => window.removeEventListener("xczs:file-tags", selected);
  }, []);
  useEffect(() => { if (action !== "tags") setTarget(null); }, [action]);
  if (action !== "tags" || !target) return null;
  return <section className="xcss-content-panel file-tag-bar" aria-label={t("文件标签", "File tags")}>
    <span className="file-tag-target">{target.name}</span>
    <FileTagRow key={target.path} client={client} target={target} />
  </section>;
}

function FileTagRow({ client, target }: {
  client: AdministratorApiClient; target: FileTagTarget;
}) {
  const [file, setFile] = useState<ListedTags | null>(null);
  const [assigned, setAssigned] = useState<TagPage | null>(null);
  const [choices, setChoices] = useState<TagPage | null>(null);
  const [cursor, setCursor] = useState<string | null>(null);
  const [choiceCursor, setChoiceCursor] = useState<string | null>(null);
  const [pending, setPending] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState("");
  const [version, setVersion] = useState(0);
  const busy = useRef(false);
  const [added, setAdded] = useState<TagRow[]>([]);
  useEffect(() => {
    let live = true;
    const controller = new AbortController();
    setLoading(true);
    setFile(null);
    setError("");
    (async () => {
      const current = await tagRequest(
        client, `/file?${new URLSearchParams({ path: target.path })}`,
        listedTags, undefined, undefined, controller.signal,
      );
      if (!current.file_id) {
        if (live) { setFile(current); setAssigned(null); setChoices(null); }
        return;
      }
      const tagged = await tagRequest(
        client, `/files/${current.file_id}/tags${cursorQuery(cursor)}`,
        tagsPage, undefined, undefined, controller.signal,
      );
      const available = await tagRequest(
        client, `/tags${cursorQuery(choiceCursor)}`,
        tagsPage, undefined, undefined, controller.signal,
      );
      if (live) {
        setFile(current); setAssigned(tagged); setChoices(available);
        if (!current.tags_has_more)
          setAdded(previous => previous.filter(tag => current.tags.some(value => value.id === tag.id)));
      }
    })().catch((cause) => { if (live) setError(tagFailure(cause)); })
      .finally(() => { if (live) setLoading(false); });
    return () => { live = false; controller.abort(); };
  }, [client, target, cursor, choiceCursor, version]);

  async function indexLibrary() {
    if (busy.current || loading) return;
    busy.current = true;
    setPending(true);
    setError("");
    try {
      await tagRequest(client, "/scan", scanResponse, "POST");
      setVersion(value => value + 1);
      window.dispatchEvent(new Event("xczs:tags-changed"));
    } catch {
      setError(t("扫描结果未能确认，请查看文件库状态后再决定是否重试。", "The scan result could not be confirmed. Check Library status before retrying."));
    } finally { busy.current = false; setPending(false); }
  }

  async function mutate(id: number, action: "add" | "remove") {
    if (!file?.file_id || busy.current || loading) return;
    busy.current = true;
    setPending(true);
    setError("");
    try {
      await tagRequest(client, "/file-tags", empty, "POST", {
        file_ids: [file.file_id], tag_ids: [id], action,
      });
      const tag = assigned?.tags.find(tag => tag.id === id) ?? choices?.tags.find(tag => tag.id === id) ?? added.find(tag => tag.id === id);
      setAdded(previous => action === "add" && tag
        ? [...previous.filter(value => value.id !== id), tag]
        : previous.filter(value => value.id !== id));
      if (action === "remove")
        setAssigned(previous => previous ? { ...previous, tags: previous.tags.filter(tag => tag.id !== id) } : previous);
      setLoading(true);
      setCursor(null);
      setVersion((value) => value + 1);
      window.dispatchEvent(new Event("xczs:tags-changed"));
    } catch (cause) { setError(tagFailure(cause)); }
    finally { busy.current = false; setPending(false); }
  }
  const disabled = pending || loading || !file?.file_id;
  const atLimit = Boolean(file && (file.tags_has_more || file.tags.length >= 16));
  const overLimit = Boolean(file && (file.tags_has_more || file.tags.length > 16));
  const selected = new Map<number, TagLabel>([...(file?.tags ?? []), ...(assigned?.tags ?? []), ...added].map(tag => [tag.id, tag]));
  const labels = overLimit ? assigned?.tags ?? [] : [...selected.values(), ...(choices?.tags ?? []).filter(tag => !selected.has(tag.id))];
  return <div className="file-tag-editor">
    {error && <ErrorState onRetry={() => setVersion(value => value + 1)}>{error}</ErrorState>}
    {loading && <LoadingState />}
    {!loading && file && !file.file_id && <div role="status">
      <p>{t("该文件尚未索引。添加标签前需要扫描整个文件库，这可能需要较长时间。", "This file is not indexed. Scan the entire file library before adding tags; this may take a while.")}</p>
      <Button disabled={pending} onClick={() => void indexLibrary()}>{pending ? t("正在扫描整个文件库…", "Scanning the entire library…") : t("扫描整个文件库", "Scan entire library")}</Button>
      <a href="#status">{t("查看文件库状态", "Library status")}</a>
    </div>}
    {file?.file_id && <span className="file-tag-count">{file.tags_has_more
      ? t("标签超过 16 个，请先移除至少于 16 个。", "More than 16 tags. Remove tags until fewer than 16 remain.")
      : t("标签：{0}/16", "Tags: {0}/16", [String(file.tags.length)])}</span>}
    {atLimit && !file?.tags_has_more && <span>{t("每个文件最多 16 个标签，请先移除后再添加。", "Each file can have at most 16 tags. Remove a tag before adding another.")}</span>}
    {assigned && choices && <>
      <div className="file-tag-options" role="group" aria-label={t("选择标签", "Select tags")}>
        {labels.map(tag =>
          <Button key={tag.id} className="file-tag-choice" disabled={disabled || (atLimit && !selected.has(tag.id))} aria-pressed={selected.has(tag.id)}
            aria-label={selected.has(tag.id)
              ? t("移除标签 {0}", "Remove tag {0}", [tag.name])
              : t("添加标签 {0}", "Add tag {0}", [tag.name])}
            onClick={() => void mutate(tag.id, selected.has(tag.id) ? "remove" : "add")}>
            <span className="file-tag-label" style={tag.color ? { borderInlineStartColor: tag.color } : undefined}>{tag.name}</span>
          </Button>)}
        {!assigned.tags.length && !choices.tags.length && <span>{t("请先在标签管理中创建标签。", "Create tags in Manage tags first.")}</span>}
      </div>
      {(assigned.previous_cursor || assigned.next_cursor) && <div className="search-tag-pager" aria-label={t("已添加标签分页", "Assigned tag pages")}>
        <Button disabled={!assigned.previous_cursor || disabled} onClick={() => setCursor(assigned.previous_cursor)}>{t("已添加标签：上一页", "Assigned tags: previous page")}</Button>
        <Button disabled={!assigned.next_cursor || disabled} onClick={() => setCursor(assigned.next_cursor)}>{t("已添加标签：下一页", "Assigned tags: next page")}</Button>
      </div>}
      {!overLimit && (choices.previous_cursor || choices.next_cursor) && <div className="search-tag-pager" aria-label={t("可用标签分页", "Available tag pages")}>
        <Button disabled={!choices.previous_cursor || disabled} onClick={() => setChoiceCursor(choices.previous_cursor)}>{t("可用标签：上一页", "Available tags: previous page")}</Button>
        <Button disabled={!choices.next_cursor || disabled} onClick={() => setChoiceCursor(choices.next_cursor)}>{t("可用标签：下一页", "Available tags: next page")}</Button>
      </div>}
    </>}
  </div>;
}
