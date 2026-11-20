/**
 * markstream heading AST node (see stream-markdown-parser's HeadingNode).
 * Mirrored as the structural contract `HeadingNode` consumes; the parser always
 * provides `text` as plain text, so no DOM-based slug computation is needed.
 */
export interface MarkdownHeadingAst {
  type: 'heading'
  level: number
  text: string
  attrs?: Record<string, string | boolean>
  children: { type: string; raw: string; [key: string]: unknown }[]
  raw: string
}
