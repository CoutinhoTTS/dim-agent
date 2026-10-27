"""A branch part in Python: it rebuilds from committed history, follows live
events in order and, after the first turn ends, writes one marked input to
its own branch naming the branch and the events it has seen.
`python_plugin` registers it."""

from rutis import define_plugin


async def apply(ctx, config):
    dispatcher = ctx.use("dispatcher")
    tree = ctx.use("treeCommands")
    identity = await ctx.use("timelineIdentity").get()
    session, branch = identity["sessionId"], identity["branchId"]
    seen = []
    state = {"written": False}

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

    async def handler(delivery):
        if "replay" in delivery:
            for event in delivery["replay"]:
                step(event, False)
            return
        written = step(delivery["live"], True)
        if written is not None:
            await written

    await dispatcher.consume("python-branch-fixture", ["input/message", "turn/start", "turn/end"], handler)


part = define_plugin(
    apply, inject=["dispatcher", "timelineIdentity", "treeCommands"], name="PythonBranchPart"
)
