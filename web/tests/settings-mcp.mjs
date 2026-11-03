import assert from "node:assert/strict"
import { readFileSync } from "node:fs"
import { registerHooks, stripTypeScriptTypes } from "node:module"
import test from "node:test"

registerHooks({
  resolve(specifier, context, next) {
    try { return next(specifier, context) } catch (error) {
      if ((specifier.startsWith("./") || specifier.startsWith("../")) && !/\.(ts|vue)$/.test(specifier)) return next(`${specifier}.ts`, context)
      throw error
    }
  },
  load(url, context, next) {
    if (url.endsWith(".ts")) return { format: "module", source: stripTypeScriptTypes(readFileSync(new URL(url), "utf8"), { mode: "transform" }), shortCircuit: true }
    return next(url, context)
  },
})

const { readMcp, readMcpConfiguration } = await import("../src/rpc/general-settings.ts")

function catalog(invalid) {
  return {
    userRevision: "user:1", projectRevision: null,
    userApplication: { error: null, cleanupError: null }, pluginApplication: null, projectApplication: null,
    servers: [{ id: "docs", source: "user", enabled: true, transport: "stdio" }],
    ...(invalid === undefined ? {} : { invalid }),
  }
}

test("a broken entry is read apart from the servers", () => {
  const error = "MCP server broken: field type must be http or sse; ws is not a supported transport"
  const view = readMcp(catalog([{ id: "broken", source: "project", error }]))
  assert.deepEqual(view.servers.map((server) => server.id), ["docs"])
  assert.deepEqual(view.invalid, [{ id: "broken", source: "project", error }])
})

test("a reply without the invalid list still reads its servers", () => {
  const view = readMcp(catalog(undefined))
  assert.deepEqual(view.servers.map((server) => server.id), ["docs"])
  assert.deepEqual(view.invalid, [])
})

test("settings show referenced and unset variable names", () => {
  const view = readMcp({
    ...catalog(undefined),
    servers: [{ id: "docs", source: "user", enabled: true, transport: "http",
      referencedVariables: ["MCP_HOST", "MCP_TOKEN"], unsetVariables: ["MCP_TOKEN"] }],
  })
  assert.deepEqual(view.servers[0].referencedVariables, ["MCP_HOST", "MCP_TOKEN"])
  assert.deepEqual(view.servers[0].unsetVariables, ["MCP_TOKEN"])
  const legacy = readMcp(catalog(undefined))
  assert.deepEqual(legacy.servers[0].referencedVariables, [])
  assert.deepEqual(legacy.servers[0].unsetVariables, [])
})

test("a configuration view names variables without values", () => {
  const configuration = readMcpConfiguration({
    type: "http", endpoint: "https://${MCP_HOST}", headerNames: ["Authorization"],
    authConfigured: true, oauthConfigured: false, oauth: null,
    referencedVariables: ["MCP_HOST", "MCP_TOKEN"], unsetVariables: ["MCP_TOKEN"],
  })
  assert.deepEqual(configuration.referencedVariables, ["MCP_HOST", "MCP_TOKEN"])
  assert.deepEqual(configuration.unsetVariables, ["MCP_TOKEN"])
  const legacy = readMcpConfiguration({ type: "stdio", command: "server", argsCount: 0, cwd: null, envNames: [] })
  assert.deepEqual(legacy.referencedVariables, [])
  assert.deepEqual(legacy.unsetVariables, [])
})
