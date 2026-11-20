import type { Component } from 'vue'

/**
 * Toolbar labels for the user-message actions row. The host resolves each one
 * through its own catalog; the package carries no desktop translation keys.
 */
export interface ChatUiTimelineUserActionLabels {
  withdraw: string
  copy: string
  edit: string
  fork: string
  branch: string
}

/** Host icon components; the package does not depend on an icon library. */
export interface ChatUiTimelineUserActionIcons {
  withdraw: Component
  copy: Component
  check: Component
  edit: Component
  fork: Component
  branch: Component
}

/**
 * Presentational flags for one user-message actions row. The host owns the
 * interaction controller and passes the already-resolved action state:
 * `canEdit` includes the "has text" requirement, `canFork`/`canBranch` exclude
 * the popout window, and `actionPending` disables write actions.
 */
export interface ChatUiTimelineUserActionsProps {
  timeText: string
  canWithdraw: boolean
  canEdit: boolean
  canFork: boolean
  canBranch: boolean
  branchDisabled?: boolean
  actionPending?: boolean
  copied?: boolean
  labels: ChatUiTimelineUserActionLabels
  icons: ChatUiTimelineUserActionIcons
  buttonComponent?: Component | string
  tooltipProviderComponent: Component
  tooltipComponent: Component
  tooltipTriggerComponent: Component
  tooltipContentComponent: Component
}
