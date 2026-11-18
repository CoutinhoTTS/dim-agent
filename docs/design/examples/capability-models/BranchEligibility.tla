-------------------- MODULE BranchEligibility --------------------
EXTENDS Naturals, Sequences, FiniteSets, TLC
\* One line's eligibility. The roles the primary line separates are all here:
\* `root` is the line with no parent, `primary` says the line is the Session's
\* conversation, `opener` says it has opened lines of its own, and `source`
\* says whether the tool, the user or a plugin opened it. `Move` is the line
\* becoming primary, which is the status note the Session writes for it.
\* What this model checks is what a line may do, never when it should do it:
\* opening policy is not modelled here.
VARIABLE s
Init == \E source\in{"tool","user","plugin"},primary\in BOOLEAN,notice\in BOOLEAN:
 /\ (source="plugin" => ~primary)
 /\ s=[root|->FALSE,primary|->primary,opener|->FALSE,source|->source,notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
 ended|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,notes|->0,blocks|-><<>>,roster|->{1,2},opened|->{},endedNotice|->{},externalEnds|->{}]
\* The root is not primary in every state: a move to another line leaves it open
\* and no longer primary, and that is the state in which it is a valid target.
RootInit == \E primary\in BOOLEAN,notice\in BOOLEAN:
 s=[root|->TRUE,primary|->primary,opener|->TRUE,source|->"root",notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
 ended|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,notes|->0,blocks|-><<>>,roster|->{1,2},opened|->{},endedNotice|->{},externalEnds|->{}]
CanOpen == s.primary /\ s.source#"plugin"
CanEndSelf == ~s.root /\ ~s.primary
\* The root is a target whenever it is not the primary line, so the move back to
\* where the Session started is a `Move` like any other; a plugin line never is.
CanBeMovedTo == ~s.primary /\ s.source#"plugin" /\ ~s.ended
Move == /\ CanBeMovedTo /\ s'=[s EXCEPT !.primary=TRUE, !.notes=@+1]
ParentAdvance == /\ s.head<3 /\ s'=[s EXCEPT !.head=@+1]
Read(after,through,ok) == /\ after<=s.head /\ through<=s.head /\ through>=after
 /\ LET floor==IF s.root THEN after ELSE IF after<s.cut THEN s.cut ELSE after
 IN /\ through>=floor /\ s'=[s EXCEPT !.cursor=IF ok THEN through ELSE @]
Block == /\ ~s.root /\ ~s.ended /\ ~s.held /\ s.head>s.opening /\ s.head>s.cursor /\ s.head>s.reported
 /\ s'=[s EXCEPT !.held=TRUE, !.reported=s.head, !.blocks=Append(@,[head|->s.head,cursor|->s.cursor,opening|->s.opening])]
ConsumeBlock == s'=[s EXCEPT !.held=FALSE]
Start == /\ ~s.turn /\ ~s.ended /\ ~s.recovery /\ s.idle<1
 /\ s'=[s EXCEPT !.turn=TRUE, !.steered=FALSE, !.ownEnd=FALSE]
Steer(ok) == /\ s.turn /\ s'=[s EXCEPT !.steered=@\/ok]
Pending(b) == s'=[s EXCEPT !.pending=b]
OwnEnd == /\ s.turn /\ CanEndSelf /\ s'=[s EXCEPT !.ownEnd=TRUE, !.ended=TRUE]
EndTurn == /\ s.turn /\ s.idle<1
 /\ s'=[s EXCEPT !.turn=FALSE, !.idle=IF s.source="tool" /\ ~s.primary /\ s.notice /\ ~s.steered /\ ~s.ownEnd /\ ~s.pending THEN @+1 ELSE @]
Restore == /\ ~s.recovery /\ s'=[s EXCEPT !.recovery=TRUE, !.turn=FALSE]
\* The opener ends a line it opened; the roster is the opener's own. The root
\* is an opener like any other line, and the primary line may end the lines it
\* opened; neither ever ends itself (`OwnEnd`).
ChildEnd(i,own) == /\ s.opener /\ i\in s.roster
 /\ s'=[s EXCEPT !.roster=@\{i}, !.endedNotice=IF own THEN @ ELSE @\cup{i}, !.externalEnds=IF own THEN @ ELSE @\cup{i}]
Open(i) == /\ CanOpen /\ i\in 1..2 /\ i\notin s.opened
 /\ s'=[s EXCEPT !.opened=@\cup{i}, !.opener=TRUE]
Next == Move\/ParentAdvance\/(\E a,t\in 0..3,b\in BOOLEAN:Read(a,t,b))\/Block\/ConsumeBlock\/Start
 \/(\E b\in BOOLEAN:Steer(b)\/Pending(b))\/OwnEnd\/EndTurn\/Restore\/(\E i\in 1..2,b\in BOOLEAN:ChildEnd(i,b))\/Open(1)\/Open(2)
Spec == (Init\/RootInit) /\ [][Next]_s
ParentBlockHasNewActivity == \A n\in 1..Len(s.blocks):s.blocks[n].head>s.blocks[n].cursor /\ s.blocks[n].head>s.blocks[n].opening
NoRootParentBlock == s.root=>s.blocks= <<>>
EndNoticeAndRosterAtomic == s.endedNotice=s.externalEnds /\ s.endedNotice\cap s.roster={}
IdleEligibility == [][s.idle'>s.idle=>s.source="tool" /\ ~s.primary /\ s.notice /\ ~s.steered /\ ~s.ownEnd /\ ~s.pending /\ s.turn]_s
RestoreNoOldIdle == [][Restore=>s.idle'=s.idle]_s
ExplicitReadCanReset == [][\A a,t\in 0..3:Read(a,t,TRUE)=>s.cursor'=t]_s
\* A line never ends itself while it is the root or the primary line. What is
\* out of scope here: an end cascaded from an ancestor. The design's guard for
\* that is the ancestor check on the end entry, not this model.
RootAndPrimaryEndNotThemselves == (s.root\/s.primary)=>~s.ended
\* A plugin line never opens a line and never becomes primary. The two together
\* are what the design says of a plugin-only line: its creator owns it, and the
\* primary line is a conversation line of the tool or the user.
PluginLineIsLeaf == s.source="plugin"=>Cardinality(s.opened)=0
PluginLineNeverPrimary == s.source="plugin"=>~s.primary
\* Only the primary line opens lines, and never a plugin line.
OnlyPrimaryOpens == [][Cardinality(s.opened')>Cardinality(s.opened)=>s.primary /\ s.source#"plugin"]_s
\* Moving the primary line writes one status note and changes no roster: the
\* lines a line opened stay its own.
MoveKeepsRoster == [][Move=>s.notes'=s.notes+1 /\ s.roster'=s.roster]_s
=============================================================================
