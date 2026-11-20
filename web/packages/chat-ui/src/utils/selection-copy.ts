/**
 * Return the current selection text only when it intersects `element`, so a
 * copy action in one timeline row cannot pick up text selected elsewhere.
 */
export function getSelectedTextInElement(element: HTMLElement | null): string {
  const selection = window.getSelection()
  if (!element || !selection || selection.isCollapsed || selection.rangeCount === 0) {
    return ''
  }

  const text = selection.toString()
  if (!text) {
    return ''
  }

  for (let index = 0; index < selection.rangeCount; index += 1) {
    const range = selection.getRangeAt(index)
    try {
      if (range.intersectsNode(element)) {
        return text
      }
    } catch {
      // Fall back to endpoint containment for unusual detached ranges.
    }
  }

  if (element.contains(selection.anchorNode) || element.contains(selection.focusNode)) {
    return text
  }

  return ''
}
