export type * from "../../../npm/client-protocol"

export const PROTOCOL_VERSION = 4
/** The root line's id: the line of a Session that has no parent. */
export const ROOT_BRANCH_ID = "main"
/**
 * The root, standing in for the default branch at a site that does not read the
 * Session's default branch. It is always the root: a site that reads it does not
 * follow a Session whose default branch has moved.
 */
export const DEFAULT_BRANCH_ID = ROOT_BRANCH_ID

export type Delegation = {
  id: string
  title: string
  prompt: string
  at: number
  state: "starting" | "running" | "completed" | "failed" | "cancelled"
  memberId?: string
  agentId?: string
  result?: string
  reason?: string
}
