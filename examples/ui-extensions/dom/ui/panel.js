/** @type {import('../../../../npm/ui').PanelModule['mount']} */
export function mount(root, context) {
  const style = document.createElement("style")
  style.textContent = ":host{display:block;padding:16px;background:var(--dim-ui-background);color:var(--dim-ui-foreground);font-family:var(--dim-ui-font-family)}p{overflow-wrap:anywhere}button{font:inherit}pre{white-space:pre-wrap;overflow-wrap:anywhere}"
  const heading = document.createElement("h2")
  const binding = document.createElement("p")
  binding.textContent = `${context.environment.sessionId} / ${context.environment.branchId}`
  const status = document.createElement("p")
  status.setAttribute("role", "status")
  const button = document.createElement("button")
  button.type = "button"
  button.textContent = "Read recent history"
  const history = document.createElement("pre")
  root.append(style, heading, binding, status, button, history)
  context.observeEnvironment(environment => { heading.textContent = environment.locale.startsWith("zh") ? "会话状态" : "Conversation activity" })
  context.services.session.state.observe(activity => {
    status.textContent = `Branch: ${activity.branch.stopping ? "stopping" : activity.branch.running ? "running" : "idle"}; Session: ${activity.sessionRunning ? "working" : "idle"}`
  })
  button.addEventListener("click", async () => {
    button.disabled = true
    try {
      const page = await context.services.session.history.read({ limit: 20 })
      if (!context.signal.aborted) history.textContent = JSON.stringify(page.events, null, 2)
    } catch (error) {
      if (!context.signal.aborted) history.textContent = error.message
    } finally { if (!context.signal.aborted) button.disabled = false }
  }, { signal: context.signal })
  return () => root.replaceChildren()
}
