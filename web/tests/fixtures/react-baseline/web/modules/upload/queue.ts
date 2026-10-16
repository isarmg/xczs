import { t } from "../../platform.ts";
/** A small FIFO with O(1) dequeue and cancellation. Cancelled entries are
 * skipped lazily and storage is compacted occasionally.
 */
export function createUploadQueue<T>() {
  let entries: { active: boolean; value: T }[] = [];
  let head = 0;
  let size = 0;

  function enqueue(value: T) {
    const entry = { active: true, value };
    entries.push(entry);
    size++;
    return entry;
  }

  function cancel(entry: { active: boolean } | null) {
    if (!entry?.active) return false;
    entry.active = false;
    size--;
    compact();
    return true;
  }

  function dequeue() {
    while (head < entries.length) {
      const entry = entries[head++];
      if (!entry.active) continue;
      entry.active = false;
      size--;
      compact();
      return entry.value;
    }
    compact(true);
    return null;
  }

  function compact(force = false) {
    if (!force && (head < 256 || head * 2 < entries.length)) return;
    entries = entries.slice(head);
    head = 0;
  }

  return Object.freeze({
    enqueue,
    cancel,
    dequeue,
    get size() {
      return size;
    },
  });
}

/** Keep a bounded FIFO of lightweight UI history entries.
 */
export function createBoundedHistory<T>(
  limit: number,
  onEvict: (entry: T) => void,
) {
  if (!Number.isSafeInteger(limit) || limit <= 0) {
    throw new TypeError(
      t(
        "历史记录数量限制必须为正整数",
        "History limit must be a positive integer",
      ),
    );
  }

  const entries: T[] = [];
  let evicted = 0;

  function add(value: T) {
    entries.push(value);
    while (entries.length > limit) {
      const oldest = entries.shift();
      if (oldest === undefined) break;
      onEvict(oldest);
      evicted++;
    }
    return evicted;
  }

  function remove(value: T) {
    const index = entries.indexOf(value);
    if (index < 0) return false;
    entries.splice(index, 1);
    return true;
  }

  return Object.freeze({
    add,
    remove,
    get evicted() {
      return evicted;
    },
    get size() {
      return entries.length;
    },
  });
}
