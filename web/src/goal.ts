export type GoalStatus =
  | { kind: "active" }
  | { kind: "paused"; reason: string }
  | { kind: "completed" }
  | { kind: "blocked" }
  | { kind: "none" }

/** The goal in force on one branch. */
export type Goal = { objective: string; status: GoalStatus; evaluator: boolean }

/** The method a writing request calls: `session.goal.<call>`. */
export type GoalCall = "set" | "edit" | "pause" | "resume" | "clear"

/** What `/goal` does with the words after it. */
export type GoalRequest =
  | { form: "show" }
  | { form: "objective"; objective: string }
  | { form: "set" | "edit"; objective: string }
  | { form: "pause" | "resume" | "clear" }

/** Reads `/goal`'s words the way the TUI does; `undefined` is a usage error. */
export function parseGoalRequest(words: string): GoalRequest | undefined {
  const trimmed = words.trim()
  if (trimmed === "") return { form: "show" }
  const space = trimmed.search(/\s/)
  const word = space === -1 ? trimmed : trimmed.slice(0, space)
  const rest = space === -1 ? "" : trimmed.slice(space).trim()
  switch (word) {
    case "pause":
    case "resume":
    case "clear":
      if (rest === "") return { form: word }
      break
    case "set":
    case "edit":
      return rest === "" ? undefined : { form: word, objective: rest }
  }
  return { form: "objective", objective: trimmed }
}

/** The objective a writing request carries. */
export function goalObjective(request: GoalRequest): string | undefined {
  switch (request.form) {
    case "show":
    case "pause":
    case "resume":
    case "clear":
      return undefined
    default:
      return request.objective
  }
}

/**
 * The call a request makes, given the goal in force. A typed objective edits
 * only a goal still being worked toward (active or paused); otherwise it sets
 * a new goal.
 */
export function goalCall(request: GoalRequest, goal: Goal | undefined): GoalCall | undefined {
  switch (request.form) {
    case "show": return undefined
    case "objective": return goal !== undefined && (goal.status.kind === "active" || goal.status.kind === "paused") ? "edit" : "set"
    default: return request.form
  }
}
