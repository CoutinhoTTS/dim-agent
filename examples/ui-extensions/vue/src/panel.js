import { createApp, h, shallowRef } from "vue"

/** @type {import('../../../../npm/ui').PanelModule['mount']} */
export function mount(root, context) {
  const environment = shallowRef(context.environment)
  const activity = shallowRef()
  context.observeEnvironment(value => { environment.value = value })
  context.services.session.state.observe(value => { activity.value = value })
  const container = document.createElement("div")
  container.style.cssText = "padding:16px;color:var(--dim-ui-foreground);background:var(--dim-ui-background);font-family:var(--dim-ui-font-family)"
  root.append(container)
  const app = createApp({
    setup: () => () => h("section", [
      h("h2", environment.value.locale.startsWith("zh") ? "Vue 状态面板" : "Vue activity"),
      h("p", `Branch: ${environment.value.branchId}`),
      h("output", JSON.stringify(activity.value)),
    ]),
  })
  app.mount(container)
  return () => { app.unmount(); container.remove() }
}
