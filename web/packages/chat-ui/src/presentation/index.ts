import type { InjectionKey, Ref } from 'vue'

/**
 * True while the timeline is restoring a session. Restore mounts the settle
 * window in one batch per frame; enter/appear transitions on those rows have
 * no viewer (the restore cover hides them) and only add compositor work and
 * delayed first paint.
 */
export const CHAT_UI_TIMELINE_RESTORE_ACTIVE: InjectionKey<Readonly<Ref<boolean>>>
  = Symbol('timeline-restore-active')

/** Side-chat branch owning this timeline; null/absent means the main line. */
export const CHAT_UI_TIMELINE_BRANCH_ID: InjectionKey<Readonly<Ref<string | null>>>
  = Symbol('timeline-branch-id')

/**
 * Real Session id for RPC reads when the timeline's display `sessionId` is a
 * synthetic branch key (the side chat uses `<sessionId>:<branchId>` as its
 * display scope).
 */
export const CHAT_UI_TIMELINE_DETAIL_SESSION_ID: InjectionKey<Readonly<Ref<string | null>>>
  = Symbol('timeline-detail-session-id')
