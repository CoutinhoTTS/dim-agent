# Session and branch owner operations

Proposed plugin API. IDs address the Session or branch owning the operation. Request, reply
and event names denote existing owner data unless defined here. A subscription delivers
the owner's initial observation and subsequent updates under the same ordering contract;
closing the caller releases its registration.

## Admission, opening and branch control

| Catalog entry | Public operation and ownership |
| --- | --- |
| `timelineTree`, `timelineTreeControl` | Plugins use treeCommands.create/end/info/input/list/page/query. Directory reads that distinguish ending and ended use directory({}) → [{info,head,state,endEvent?}]. Restoration, branch handle lookup, main creation and Session close remain within the tree/runtime owner. |
| `timelineFactory` | Branch creation returns the branch identity from treeCommands.create. Runtime builds, publishes and disposes its branch internally. |
| `sessionLoader` | create({group,definitionId,instanceData}) → {instanceId}; remove({instanceId}) → void. The definition and instance data are serializable. Owner-local resources are resolved at their owning node. The owning Session anchors creation. MCP supplies its connection definition and installs its owner-local driver and credentials in that scope. |
| `branchOpening` | register(completion) → void admits a receipt during opening. The callback settles with success or the original error. Opening waits for all admitted receipts with its original deadline and joins work on close. |
| `faultReporter` | report(error) → void, failure({}) → AgentError or null, observe(listener) → subscription. Retain the first fault and the existing precedence of a storage fault. |
| `submitChecks` | register(name, check) → void; check({entry,branchId,user?,mode}, cancelled) → proceed{blocks,warnings} or block{message}. The caller owns the registration until unload. Retain name order, first block and interrupted-without-write. |
| `agentLoop` | compact({}) → sequence; stopTurn({turnId,reason}) → sequence or null. Loop owns admission and writes the same facts. |
| `loopState` | state({}) → {phase,turnId?}; observe(listener) delivers the same snapshot on change. States are idle, running and stopping. |

## Prompt, configuration and files

| Catalog entry | Public operation and ownership |
| --- | --- |
| `prompt` | initialCompletion({}) → void or the original opening error. Prompt owns loading and adoption. |
| `configureControl` | configure({sources,model?}) → unchanged, adopted or superseded. Prompt preserves ordering across overlapping updates. |
| `configurationPublisher` | publish({adopted,changes,model?,toolAdoption?,inputs}) → sequence or null; awaitPublished({sequence}) → sequence. Prompt prepares input data and the publication owner performs the existing atomic adoption, registration and publication at the commit boundary. Guide adoption uses the same owner operation with its guide revision. |
| `promptSourcesBranch` | The Session language owner serves sessionLanguage.get and set({language}) → the existing language/outcome reply. The owner serializes persistence, cell replacement and configure delivery in that order. Loaded side branches retain their adoption behavior. |
| `fileReads` | binding({path}) → TextFileBinding or null; writeBinding({path}) → absent, compacted or bound; record({path,binding,established}) → void. The established sequence is captured at the original read. read/edit/write return their existing tool results and binding at the resource boundary. The branch owns the binding history, including compacted-history validation; the file node owns actual I/O. |

## Model preparation, history and tools

| Catalog entry | Public operation and ownership |
| --- | --- |
| `prepareModelHost` | prepare({turnId,requestId,history,availableTools}) → ModelSettings. Preserve pinned definitions and ordering. |
| `prepareContextHost` | prepare({request,source:{turnId,requestId,through}}, cancelled) → ModelRequest. The native caller can retain the unchanged request allocation. |
| `contextModelHost` | describe(model selection and metadata) → {contextWindow,outputTokens,thresholdTokens,algorithm?}; count(request) → tokens; contributions({model,fixed?,messages}) → {algorithm,fixed?,messages}; estimate({model,algorithm,contribution}) → tokens. Compute message contributions together. Recovery uses saved contributions under the matching algorithm. |
| `compactionStrategyHost` | describe(model facts) → {name,replacementSize}; compact(input,cancelled) → completed, skipped or failed with existing StrategyDiagnostics. Input contains history, checkpoint, request, legal cuts, trigger, budgets, prior diagnostics and model facts. Counting uses the model owner. Replay applies the committed result. |
| `historyRenderer` | Render a branch at an explicit through sequence: context({branchId,through}) → ContextHistory and renderedContext({branchId,through}) → {context,history:HistoryEntry[]}. snapshot({context,retainFrom,anchor}) → RecoverySnapshot. streamCheckpoint({branchId,through,retainFrom,batch}) returns ProjectionCheckpoint. Preserve the checkpoint and existing bounded read path. |
| `contextPreparation` | prepare({source:ContextPrepareData,projection?},cancelled) → ModelRequest; validate({requestId,request},cancelled) → void. Transfer any already-prepared projection as part of the request handoff. Use the same request identity and cutoff and avoid a second Timeline read. |
| `contextUsage` | read({through?},cancelled) → {through,contextWindow,outputTokens,thresholdTokens,estimatedInputTokens,baselineSource}. Preserve not-configured, not-ready, model and storage failures. |
| `toolBinding` | bind({requestId,through,tools,toolSources}) → void and bindCurrent({requestId,declared?}) → ToolDefinition[] pin the tool set; definitions({requestId}) returns ToolDefinition[]; transfer({from,to}) and release({requestId}) delimit retention. The owner claims and invokes the selected handler under its pinned request and call ID. |
| `toolExecutor` | definitions({requestId}) → ToolDefinition[]; open({parentCallId,context},cancelled) → gateId; invoke({gateId,call}) → ToolResult; close({gateId}) joins children and returns their ordered ToolEffect[]. The owner validates the current batch and parent, releases the gate on interruption and settles children before the parent. |

Implementation order is opening/admission, branch controls, Prompt/configuration, tool
handoff, then model preparation and recovery. Reuse the [Timeline contract](../../architecture/rust-agent/time-bus.md),
[Loop](../../architecture/rust-agent/plugins/Loop.md), [Prompt](../../architecture/rust-agent/plugins/Prompt.md),
[ContextPreparation](../../architecture/rust-agent/plugins/ContextPreparation.md) and
[ToolExecution](../../architecture/rust-agent/plugins/ToolExecution.md) facts and batch
boundaries. Wire callbacks run outside commit locks. Verify unchanged idle/stop behavior,
opening failure, concurrent configuration, recovery, read counts, pinned generation,
and unload during a nested call with native and foreign callers.
