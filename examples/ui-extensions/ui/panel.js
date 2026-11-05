/** @type {import('../../../npm/ui').PanelModule['mount']} */
export function mount(root, context) {
  root.textContent = "Loading…"
  async function load() {
    const response = await fetch(new URL("./panel.html", import.meta.url), { signal: context.signal })
    if (!response.ok) throw new Error(`Panel load failed: ${response.status}`)
    const html = await response.text()
    if (context.signal.aborted) return
    root.innerHTML = html
    const style = document.createElement("link")
    style.rel = "stylesheet"
    style.href = new URL("./panel.css", import.meta.url).href
    root.prepend(style)
    const heading = root.querySelector("h2")
    const status = root.querySelector('[role="status"]')
    const button = root.querySelector("button")
    const history = root.querySelector("pre")
    root.querySelector(".binding").textContent = `${context.environment.sessionId} / ${context.environment.branchId}`
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
  }
  void load().catch(error => { if (!context.signal.aborted) root.textContent = error.message })
  return () => root.replaceChildren()
}
