"""A Python plugin as one row: it registers its Session part and its branch
part with `partRegistry`, and each Session and branch mounts a copy."""

import python_branch_part
import python_session_part

inject = ["partRegistry"]
provides = {}


async def apply(ctx, config):
    parts = ctx.use("partRegistry")
    await parts.registerDefinition("session", "PythonSessionPart", python_session_part.part)
    await parts.registerDefinition("branch", "PythonBranchPart", python_branch_part.part)
