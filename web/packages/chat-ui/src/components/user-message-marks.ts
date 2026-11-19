/**
 * One serialized user-message mark annotation. The host parses the mark blocks
 * and keeps the inspect payload; the package only renders the presentational
 * fields and reports which annotation was opened.
 */
export interface ChatUiTimelineUserMark {
  num: number
  selector: string
  textLabel?: string
  notes: readonly string[]
  /** The annotation can be opened in the host; false renders a static card. */
  interactive?: boolean
}
