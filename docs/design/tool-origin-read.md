# Tool origin read

ToolExecution owns the nonmutating `session.tool.get` method. Parameters are
`sessionId`, `branchId`, `requestId`, `callId` and optional `through`. The reply is
`branchId`, the fixed published `through`, and nullable `call`. A present call is
the existing `ToolUseData` object: its turn, request and complete `ToolCall`.
Null means no committed complete call has that identity at the cutoff; it does
not mean an empty argument object. A partial call that never supplied arguments
can have a terminal result and no complete call.

The Session part ToolMethods declares TreeCommands and SessionMethods. It queries
the addressed branch's own facts, including ended branches, by request and call id,
selects only the matching data and retains no event list. It never invokes the
historical tool. Request cancellation ends the query.

Clients keep request and call identity on a result whose origin is outside the
transcript window. When displaying that row they resolve its origin with this
method and install the returned name and arguments in that existing row. They do
not extend the transcript to find it. Reads belong to the attachment that started
them; replacement and row eviction discard their results. A failure is shown and
can be retried by reopening the conversation.

## Rule check

ToolMethods is a stateless Session read part, not a Capability. It owns no branch
business state, writes no fact, subscribes to no event, starts no background task
and introduces no context input. Its only work belongs to the admitted request.
It uses the existing facade/property-query mechanism and typed call object.
These choices satisfy principles §4 rules 1–9 and §1, and ADR 002 D1–D5; before
callbacks, execution, configuration and I/O effect settlement do not apply.

## Requirements raised to mechanism owners

None. Timeline's field and identity queries suffice. The client's requirement is
owned by [the bounded-window client contract](clients.md#opening-a-conversation-with-a-bounded-window).
