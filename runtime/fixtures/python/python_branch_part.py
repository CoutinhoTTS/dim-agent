"""A branch part in Python: it subscribes first, folds the branch's facts
through the cutoff of its own read, follows live events in order and, after the
first turn ends, writes one marked input to its own branch naming the branch
and the events it has seen. `python_plugin` registers it."""

from rutis import define_plugin

PAGE = 1024


async def apply(ctx, config):
    dispatcher = ctx.use("dispatcher")
    timeline = ctx.use("timeline")
    tree = ctx.use("treeCommands")
    identity = await ctx.use("timelineIdentity").get()
    session, branch = identity["sessionId"], identity["branchId"]
    seen = []
    state = {"written": False}
    covered = None
    held = []

    def step(event, live):
        key = f"{event['type']}/{event['name']}"
        seen.append(key)
        if key == "input/message" and "py-marker" in str(event["data"]):
            state["written"] = True
        if live and key == "turn/end" and not state["written"]:
            state["written"] = True
            marker = f'<py-marker session="{session}" branch="{branch}" seen="{",".join(seen)}"/>'
            return tree.input(branch, {"blocks": [{"type": "raw", "text": marker}], "mode": "queue"})
        return None

    async def release(event):
        written = step(event, True)
        if written is not None:
            await written

    async def live(event):
        if covered is None:
            held.append(event)
        else:
            await release(event)

    await dispatcher.listen(
        "python-branch-fixture", ["input/message", "turn/start", "turn/end"], live
    )
    start = None
    while True:
        query = {
            "scope": "branch",
            "order_by": "sequence",
            "direction": "ascending",
            "kinds": ["input/message", "turn/start", "turn/end"],
            "limit": PAGE,
            "offset": 0,
        }
        if start is not None:
            query["from_sequence"] = start
        page = await timeline.queryWithTail(query)
        events = page["events"]
        for event in events:
            step(event, False)
        if not events or len(events) < PAGE:
            covered = page["through"]
            break
        start = int(events[-1]["sequence"]) + 1
    for event in held:
        if int(event["sequence"]) > int(covered):
            await release(event)


part = define_plugin(
    apply,
    inject=["dispatcher", "timeline", "timelineIdentity", "treeCommands"],
    name="PythonBranchPart",
)
