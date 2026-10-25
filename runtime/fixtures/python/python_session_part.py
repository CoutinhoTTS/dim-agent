"""A Session row in Python: it reads its Session's identity by name and
registers the `py_echo` tool, whose result names the Session it ran in."""

import json

inject = ["sessionIdentity", "toolRegistry"]
provides = {}


async def apply(ctx, config):
    session = await ctx.use("sessionIdentity").get()

    async def execute(call, cancelled):
        return {"content": json.dumps({
            "session": session["sessionId"],
            "callSession": call["sessionId"],
            "branch": call["branchId"],
            "text": call["arguments"].get("text"),
        })}

    await ctx.use("toolRegistry").register({
        "name": "py_echo",
        "description": "Python flow fixture",
        "inputSchema": {"type": "object"},
        "source": "extension",
        "risk": "session",
    }, execute)
