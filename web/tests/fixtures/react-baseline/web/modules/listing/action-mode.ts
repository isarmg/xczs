export type FileAction = "move" | "download" | "delete" | "rename" | "tags";

type ActionState = Readonly<{ action: FileAction | null; notice: string }>;
let state: ActionState = Object.freeze({ action: null, notice: "" });
const listeners = new Set<() => void>();

export function fileActionState(): ActionState {
  return state;
}

export function subscribeFileAction(listener: () => void) {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

export function selectFileAction(action: FileAction | null, notice = "") {
  if (state.action === action && state.notice === notice) return;
  state = Object.freeze({ action, notice });
  for (const listener of listeners) listener();
}

export function toggleFileAction(action: FileAction) {
  selectFileAction(state.action === action ? null : action);
}
