"""A Session row in Python: it reads its Session's identity by name and
provides the `py_echo` tool, whose result names the Session it ran in."""

import json

inject = ["sessionIdentity"]
provides = {"toolContribution:py_echo": {"declaration": "sync", "execute": "async"}}


class Echo:
    def __init__(self, session):
        self.session = session

    def declaration(self):
        return {
            "name": "py_echo",
            "description": "Python flow fixture",
            "inputSchema": {"type": "object"},
            "source": "extension",
            "risk": "session",
        }

    async def execute(self, call, cancelled):
        return {"content": json.dumps({
            "session": self.session["sessionId"],
            "callSession": call["sessionId"],
            "branch": call["branchId"],
            "text": call["arguments"].get("text"),
        })}


async def apply(ctx, config):
    session = await ctx.use("sessionIdentity").get()
    ctx.provide("toolContribution:py_echo", Echo(session))
