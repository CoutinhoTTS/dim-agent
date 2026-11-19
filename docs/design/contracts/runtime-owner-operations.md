# Session and branch owner operations

## Existing APIs

| Caller → owner | Reuse on main |
| --- | --- |
| Branch management and input callers → TimelineTree | `treeCommands.create/end/info/input/list/page/query`. Creation returns the branch identity. TimelineTree and the runtime own branch construction, restoration and disposal (`timelineFactory`, `timelineTreeControl`). |
| Language readers → PromptSources | `sessionLanguage.get`. |
| Prompt → PromptRegistry; guide and tool contributors → their registries | `initialPromptConfiguration.get`, `promptHost.load`, `guideEntries.register`, `toolRegistry.register`, `toolSourcesView.get/observe`. |
| Session and branch parts → their lifecycle; branch fact consumers → Dispatcher | `sessionLifecycle.activated/closing/hold/release`, `branchLifecycle.activated/closing`, `dispatcher.listen/consume/committed`. |

## New rutis APIs on existing owners

### Admission and branch control

| Caller → owner | New API |
| --- | --- |
| Branch directory readers → TimelineTree | `treeCommands.directory({}) → [{info, head, state, endEvent?}]`, including ending and ended branches. |
| MCP publication → Session loader | `sessionLoader.create({group, definitionId, instanceData}) → {instanceId}`; `remove({instanceId}) → void`. The Session anchors the instance; MCP supplies its definition and resolves its driver and credentials locally. |
| Prompt and ToolBinding → BranchOpening | `register(completion) → void`. Opening waits for admitted completions under its existing deadline and propagates their errors. |
| Fault producers and Session observers → FaultReporter | `report(error) → void`, `failure({}) → AgentError or null`, `observe(listener) → subscription`. First fault wins with existing storage-fault precedence. |
| Check providers and input admission → SubmitChecks | `register(name, check) → void`; `check({entry, branchId, user?, mode}, cancelled) → proceed{blocks, warnings} or block{message}`. Run in name order through the first block. Interrupted checks write nothing. |
| Compaction, stop and activity callers → Loop | `agentLoop.compact({}) → sequence`, `stopTurn({turnId, reason}) → sequence or null`; `loopState.state({}) → {phase, turnId?}`, `observe(listener)` delivers that snapshot. |

### Prompt, configuration and files

| Caller → owner | New API |
| --- | --- |
| Branch startup and configuration callers → Prompt | `prompt.initialCompletion({}) → void or opening error`; `configureControl.configure({sources, model?}) → unchanged, adopted or superseded`. |
| Prompt and guide adoption → ContextPreparation | `configurationPublisher.publish({adopted, changes, model?, toolAdoption?, inputs}) → sequence or null`; `awaitPublished({sequence}) → sequence`. Publication commits adoption atomically. |
| Language updates → Session language owner | Add `sessionLanguage.set({language}) → existing language/outcome reply`. The owner persists the value, replaces the cell and delivers configuration to loaded branches (`promptSourcesBranch`). |
| File read/edit/write tools → branch FileReads | `binding({path}) → TextFileBinding or null`, `writeBinding({path}) → absent, compacted or bound`, `record({path, binding, established}) → void`. The original read supplies the established sequence; file I/O stays on the file node. |

### Model preparation, history and tools

| Caller → owner | New API |
| --- | --- |
| Loop → PrepareModelHost | `prepare({turnId, requestId, history, availableTools}) → ModelSettings`, with pinned tool definitions in order. |
| ContextPreparation → PrepareContextHost | `prepare({request, source:{turnId, requestId, through}}, cancelled) → ModelRequest`. |
| Context accounting and compaction → ContextModelHost | `describe(model facts) → {contextWindow, outputTokens, thresholdTokens, algorithm?}`, `count(request) → tokens`, `contributions({model, fixed?, messages}) → {algorithm, fixed?, messages}`, `estimate({model, algorithm, contribution}) → tokens`. |
| Loop and ContextPreparation → CompactionStrategyHost | `describe(model facts) → {name, replacementSize}`; `compact(input, cancelled) → completed, skipped or failed` with StrategyDiagnostics. Input carries history, checkpoint, request, legal cuts, trigger, budgets, prior diagnostics and model facts. |
| Loop and ContextPreparation → HistoryRenderer | `context({branchId, through}) → ContextHistory`, `renderedContext({branchId, through}) → {context, history: HistoryEntry[]}`, `snapshot({context, retainFrom, anchor}) → RecoverySnapshot`, `streamCheckpoint({branchId, through, retainFrom, batch}) → ProjectionCheckpoint`. |
| Loop and ModelProvider → ContextPreparation | `prepare({source: ContextPrepareData, projection?}, cancelled) → ModelRequest`, `validate({requestId, request}, cancelled) → void`. Reuse the prepared projection at the original cutoff. |
| ContextPreparation and usage readers → ContextUsage | `read({through?}, cancelled) → {through, contextWindow, outputTokens, thresholdTokens, estimatedInputTokens, baselineSource}`. |
| Loop and ToolExecutor → ToolBinding | `bind({requestId, through, tools, toolSources}) → void`, `bindCurrent({requestId, declared?}) → ToolDefinition[]`, `definitions({requestId}) → ToolDefinition[]`, `transfer({from, to}) → void`, `release({requestId}) → void`. ToolBinding retains the pinned handler generation. |
| Nested tool callers → ToolExecutor | `definitions({requestId}) → ToolDefinition[]`, `open({parentCallId, context}, cancelled) → gateId`, `invoke({gateId, call}) → ToolResult`, `close({gateId}) → ToolEffect[]`. Close joins children in order; interruption releases the gate. |

## Objects

All methods belong to existing Session or branch owners. The draft adds no plugin, service instance or persistent business object. Loader calls return an existing instance's ID. Nested tool calls expose a temporary `gateId` for an existing parent call's child gate. Opening and registration use callbacks; observations deliver the initial state and ordered updates. Registrations end on caller unload, and callbacks run outside commit locks.
