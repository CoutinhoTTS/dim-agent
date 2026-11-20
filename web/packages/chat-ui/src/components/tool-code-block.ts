export interface ToolCodeBlockTheme {
  dark: string
  light: string
}

/** Keep the diff engine from emitting "no newline at end of file" markers. */
export function ensureTrailingNewline(text: string): string {
  return text.endsWith('\n') ? text : `${text}\n`
}

/** Strip `行号\t` prefixes from read output and drop non-numbered notices. */
export function stripToolCodeLineNumbers(text: string): string {
  return text.split(/\r?\n/)
    .filter(line => /^\d+\t/.test(line))
    .map(line => /^(\d+)\t(.*)$/.exec(line)![2])
    .join('\n')
}
