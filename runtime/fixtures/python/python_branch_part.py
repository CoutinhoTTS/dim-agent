"""A branch row in Python: it rebuilds from committed history, follows live
events in order and, after the first turn ends, writes one marked input to
its own branch naming the branch and the events it has seen."""

import asyncio

inject = ["dispatcher", "timelineIdentity", "treeCommands"]
provides = {}


async def apply(ctx, config):
    dispatcher = ctx.use("dispatcher")
    tree = ctx.use("treeCommands")
    identity = await ctx.use("timelineIdentity").get()
    session, branch = identity["sessionId"], identity["branchId"]
    seen = []
    state = {"written": False, "cutoff": 0}
    rebuilt = asyncio.Event()

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

    async def handler(event):
        await rebuilt.wait()
        if int(event["sequence"]) > state["cutoff"]:
            written = step(event, True)
            if written is not None:
                await written

    # Subscribe first, then rebuild through the committed cutoff.
    await dispatcher.listen("python-branch-fixture", ["input/message", "turn/start", "turn/end"], handler)
    state["cutoff"] = int(await dispatcher.committed())
    if state["cutoff"] > 0:
        for event in await dispatcher.local(0, state["cutoff"]):
            step(event, False)
    rebuilt.set()
