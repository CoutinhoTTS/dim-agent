import assert from "node:assert/strict"
import { fileURLToPath } from "node:url"
import { after, test } from "node:test"
import { createPinia, setActivePinia } from "pinia"
import { createServer } from "vite"

const server = await createServer({
  root: fileURLToPath(new URL("../../", import.meta.url)),
  configFile: false,
  server: { middlewareMode: true },
  appType: "custom",
})
after(() => server.close())
const { useModelsStore } = await server.ssrLoadModule("/src/stores/models.ts")
const { useConnectionStore } = await server.ssrLoadModule("/src/stores/connection.ts")

function discovery() {
  setActivePinia(createPinia())
  const connection = useConnectionStore()
  connection.hello = { instanceId: "test", serverVersion: "test", protocolVersion: 3, features: ["provider.list", "provider.catalog", "account.providers"] }
  const state = { failedMethod: undefined, collision: false }
  connection.client.call = async (method, params, read) => {
    if (method === state.failedMethod) throw new Error(`${method} is reloading`)
    if (method === "provider.list") return read({ providers: [
      { providerId: "local", displayName: "Local", enabled: true, credentialConfigured: false },
      { providerId: "disabled", displayName: "Disabled", enabled: false, credentialConfigured: true },
      ...(state.collision ? [{ providerId: "managed", displayName: "Legacy static name", enabled: true, credentialConfigured: true }] : []),
    ] })
    if (method === "account.providers") return read([{ providerId: "managed", displayName: "Managed account" }])
    if (method === "provider.catalog") return read({ catalog: {
      models: [{ id: `${params.providerId}-model`, name: `${params.providerId} model` }],
      disabledModelIds: [],
    } })
    throw new Error(`Unexpected method ${method}`)
  }
  return { models: useModelsStore(), state }
}

test("model discovery merges provider identities with the managed descriptor taking precedence", async () => {
  const { models, state } = discovery()
  state.collision = true
  await models.load()
  assert.deepEqual(models.providers, [
    { providerId: "local", name: "Local", models: [{ id: "local-model", name: "local model" }] },
    { providerId: "managed", name: "Managed account", models: [{ id: "managed-model", name: "managed model" }] },
  ])
  assert.equal(models.error, "")
})

for (const [failedMethod, expected] of [
  ["account.providers", [{ providerId: "local", name: "Local", models: [{ id: "local-model", name: "local model" }] }]],
  ["provider.list", [{ providerId: "managed", name: "Managed account", models: [{ id: "managed-model", name: "managed model" }] }]],
]) {
  test(`model discovery retains healthy models when ${failedMethod} fails and recovers on retry`, async () => {
    const { models, state } = discovery()
    state.failedMethod = failedMethod
    await models.load()
    assert.deepEqual(models.providers, expected)
    assert.equal(models.error, `${failedMethod} is reloading`)

    state.failedMethod = undefined
    await models.load()
    assert.deepEqual(models.providers, [
      { providerId: "local", name: "Local", models: [{ id: "local-model", name: "local model" }] },
      { providerId: "managed", name: "Managed account", models: [{ id: "managed-model", name: "managed model" }] },
    ])
    assert.equal(models.error, "")
  })
}
