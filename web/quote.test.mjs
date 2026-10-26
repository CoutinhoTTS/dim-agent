// The quote format both composers submit; the TUI composer checks the same cases.
import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import test from "node:test"
import { quoteInto } from "./src/features/chat/quote.ts"

const cases = JSON.parse(readFileSync(new URL("./src/features/chat/quote-cases.json", import.meta.url), "utf8"))

for (const { name, draft, message, result } of cases) {
  test(name, () => assert.equal(quoteInto(draft, message), result))
}
