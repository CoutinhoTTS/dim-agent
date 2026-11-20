export const normalizeToolName = (toolName: string | undefined): string => {
  const lowerName = (toolName ?? '').trim().toLowerCase()
  if (!lowerName) return ''
  const parts = lowerName.split(/(?:__|[./\s:-])+/).filter(Boolean)
  return parts.at(-1) ?? lowerName
}
