-------------------- MODULE BranchEligibility --------------------
EXTENDS Naturals, Sequences, FiniteSets, TLC
\* One line's eligibility, in a Session whose default branch is a position in
\* the tree. Nothing here is a role marker: a line is the root when it has no
\* parent, and its opener is that parent, so both are read from the tree
\* (`IsRoot`, `HasOpener`). `primary` says the line is the Session's default
\* branch: the line its conversation is on. `source` says where the line was
\* created from, and only that.
\*
\* This model checks what a line may do, never when it should do it. Opening
\* policy is not modelled, and in this design only the root opens lines, so no
\* line here opens one.
\*
\* `ancestorEnding` and `descendantDefault` are the two tree facts the admission
\* rules read: some ancestor's end is already on its way down, which the cascade
\* will complete, and some descendant is the default branch. The environment
\* sets each of them, and that is how the two conflicting orderings reach this
\* line: an ancestor's end can commit first, or a descendant can become the
\* default branch first. Exactly one of the two is admitted.
VARIABLE s
NoParent == "none"
\* The status note under `branch:line` that BranchLine wrote on this line: none
\* yet, the one that says the line is the default branch, or the one that says
\* it no longer is. It is a keyed block, so its latest value replaces the
\* earlier one; what it must never do is reach the root.
NoNote == "none"
DefaultNote == "default"
NotDefaultNote == "notDefault"
\* A line the user may choose as the default branch: the root, a tool line and a
\* line the user opened. A plugin line follows its creator's protocol, and a
\* line a client created before the upgrade keeps its stored `host` row, so
\* neither is selectable.
Selectable == {"tool","user"}
IsRoot == s.parent = NoParent
HasOpener == ~IsRoot
\* The root is a target whenever it does not hold the role: a move away from it
\* leaves it open and no longer the default branch, and nothing else about it
\* changes, so the user can always move back to where the Session started.
CanBeMovedTo == ~s.primary /\ (IsRoot \/ s.source\in Selectable) /\ ~s.ended /\ ~s.ending /\ ~s.ancestorEnding
\* Only a tool line ends itself: a line the user opened is ended by the user, a
\* plugin line by its creator's protocol, and the root never ends. A line that
\* is the default branch, or that has a descendant which is, is refused too,
\* because ending a line ends every line below it.
CanEndSelf == HasOpener /\ ~s.primary /\ ~s.descendantDefault /\ ~s.ancestorEnding /\ s.source="tool"
\* An end asked for from outside the line: the user's, a creator's or a
\* cascade's. The end entry checks the target only, whoever asks.
CanBeEnded == HasOpener /\ ~s.primary /\ ~s.descendantDefault /\ ~s.ended /\ ~s.ending
\* A line that has an opener, which is every line but the root. The root is
\* `RootInit` alone: a parentless tool, user, plugin or upgraded line is not a
\* state the tree can hold, and starting one here would not exercise the root.
\* A new line is never the default branch and holds no note: it becomes one by
\* a move, which is where the note is written.
Init == \E source\in{"tool","user","plugin","hostOld"},notice\in BOOLEAN:
 /\ s=[parent|->"P1",source|->source,primary|->FALSE,note|->NoNote,notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
     ended|->FALSE,ending|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,
     blocks|-><<>>,roster|->{1,2},endedNotice|->{},externalEnds|->{},
     ancestorEnding|->FALSE,descendantDefault|->FALSE]
\* The Session starts on its root, so the root is the default branch here. That
\* it later holds no role is a reachable state, not an initial one:
\* `AnotherLineTakesRole` leaves it open and no longer the default branch, which
\* is the state in which it is a target again.
RootInit == \E notice\in BOOLEAN:
 /\ s=[parent|->NoParent,source|->"root",primary|->TRUE,note|->NoNote,notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
     ended|->FALSE,ending|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,
     blocks|-><<>>,roster|->{1,2},endedNotice|->{},externalEnds|->{},
     ancestorEnding|->FALSE,descendantDefault|->FALSE]
\* Becoming the default branch. A target whose ancestor is already ending is
\* refused, because the cascade will reach it, and so is the line that already
\* holds the position, whatever line it is: the root is refused while it is the
\* default branch and accepted once another line holds the role. A move writes
\* one status note, under the identity's own key, to the line that gains the
\* role and to the one that loses it; the root gets none in either direction,
\* because its two rules, that it opens lines and that it never ends, are the
\* same whether or not it holds the role.
Move == /\ CanBeMovedTo
 /\ s'=[s EXCEPT !.primary=TRUE, !.note=IF IsRoot THEN @ ELSE DefaultNote]
\* Another line becomes the default branch, so this one stops holding the role.
\* The move is that line's own action and its own note; this line records only
\* what it loses, and the root records nothing, as when it gains the role. This
\* is the route by which a root that no longer holds the role is reached; from
\* there `Move` takes it back.
AnotherLineTakesRole == /\ s.primary
 /\ s'=[s EXCEPT !.primary=FALSE, !.note=IF IsRoot THEN @ ELSE NotDefaultNote]
\* The two tree facts. Each can arrive at any time, and only the admission
\* rules decide what follows from it.
\* The root has no ancestor, so no ancestor's end can be on its way to it.
BeginAncestorEnd == /\ HasOpener /\ ~s.ancestorEnding /\ ~s.ended /\ s'=[s EXCEPT !.ancestorEnding=TRUE]
BeginDescendantDefault == /\ ~s.descendantDefault /\ ~s.primary /\ ~s.ended
 /\ s'=[s EXCEPT !.descendantDefault=TRUE]
ParentAdvance == /\ s.head<2 /\ s'=[s EXCEPT !.head=@+1]
Read(after,through,ok) == /\ after<=s.head /\ through<=s.head /\ through>=after
 /\ LET floor==IF IsRoot THEN after ELSE IF after<s.cut THEN s.cut ELSE after
 IN /\ through>=floor /\ s'=[s EXCEPT !.cursor=IF ok THEN through ELSE @]
Block == /\ HasOpener /\ ~s.ended /\ ~s.held /\ Len(s.blocks)<2
 /\ s.head>s.opening /\ s.head>s.cursor /\ s.head>s.reported
 /\ s'=[s EXCEPT !.held=TRUE, !.reported=s.head, !.blocks=Append(@,[head|->s.head,cursor|->s.cursor,opening|->s.opening])]
ConsumeBlock == s'=[s EXCEPT !.held=FALSE]
Start == /\ ~s.turn /\ ~s.ended /\ ~s.recovery /\ s.idle<1
 /\ s'=[s EXCEPT !.turn=TRUE, !.steered=FALSE, !.ownEnd=FALSE]
Steer(ok) == /\ s.turn /\ s'=[s EXCEPT !.steered=@\/ok]
Pending(b) == s'=[s EXCEPT !.pending=b]
OwnEnd == /\ s.turn /\ CanEndSelf /\ s'=[s EXCEPT !.ownEnd=TRUE, !.ending=TRUE]
EndFromOutside == /\ CanBeEnded /\ s'=[s EXCEPT !.ending=TRUE, !.turn=FALSE]
FinishEnd == /\ s.ending /\ s'=[s EXCEPT !.ending=FALSE, !.ended=TRUE]
EndTurn == /\ s.turn /\ s.idle<1
 /\ s'=[s EXCEPT !.turn=FALSE,
     !.idle=IF s.source="tool" /\ ~s.primary /\ s.notice /\ ~s.steered /\ ~s.ownEnd /\ ~s.pending THEN @+1 ELSE @]
Restore == /\ ~s.recovery /\ s'=[s EXCEPT !.recovery=TRUE, !.turn=FALSE]
\* The opener ends a line it opened, and the roster is the opener's own. The
\* default branch may end the lines it opened; no line ever ends itself this way.
ChildEnd(i,own) == /\ HasOpener /\ i\in s.roster
 /\ s'=[s EXCEPT !.roster=@\{i}, !.endedNotice=IF own THEN @ ELSE @\cup{i}, !.externalEnds=IF own THEN @ ELSE @\cup{i}]
Next == Move\/AnotherLineTakesRole\/BeginAncestorEnd\/BeginDescendantDefault\/ParentAdvance
 \/(\E a,t\in 0..2,b\in BOOLEAN:Read(a,t,b))\/Block\/ConsumeBlock\/Start
 \/(\E b\in BOOLEAN:Steer(b)\/Pending(b))\/OwnEnd\/EndFromOutside\/FinishEnd\/EndTurn\/Restore
 \/(\E i\in 1..2,b\in BOOLEAN:ChildEnd(i,b))
Spec == (Init\/RootInit) /\ [][Next]_s
ParentBlockHasNewActivity == \A n\in 1..Len(s.blocks):s.blocks[n].head>s.blocks[n].cursor /\ s.blocks[n].head>s.blocks[n].opening
\* The root has nothing above it: a line with no opener gets no parent block.
NoRootParentBlock == IsRoot=>s.blocks= <<>>
EndNoticeAndRosterAtomic == s.endedNotice=s.externalEnds /\ s.endedNotice\cap s.roster={}
IdleEligibility == [][s.idle'>s.idle=>s.source="tool" /\ ~s.primary /\ s.notice /\ ~s.steered /\ ~s.ownEnd /\ ~s.pending /\ s.turn]_s
RestoreNoOldIdle == [][Restore=>s.idle'=s.idle]_s
ExplicitReadCanReset == [][\A a,t\in 0..2:Read(a,t,TRUE)=>s.cursor'=t]_s
\* A line never ends itself while it is the root or the default branch. Out of
\* scope here: an end cascaded from an ancestor, which the ancestor check on the
\* end entry guards, not this model.
RootAndPrimaryEndNotThemselves == (IsRoot\/s.primary)=>~s.ended
\* A line the user opened is never ended by its own `end`: the user ends it.
UserLineNeverEndsItself == s.source="user"=>~s.ownEnd
\* A plugin line is never the default branch, and neither is a line a client
\* created before the upgrade: nothing rewrites either kind.
PluginNeverDefault == s.source="plugin"=>~s.primary
UpgradedHostLineNeverDefault == s.source="hostOld"=>~s.primary
\* The default branch is an open conversation line: it is the root, a tool line
\* or a line the user opened, and it is open. An ended, ending or plugin line is
\* never the line the Session's conversation is on.
DefaultIsOpenConversationLine == s.primary=>((IsRoot \/ s.source\in Selectable) /\ ~s.ended /\ ~s.ending)
\* Gaining the role writes one status note to a line that has an opener. The root
\* is the exception the next invariant covers: its two rules do not change with
\* the role.
MoveWritesOneNote == [][Move /\ ~IsRoot=>s.note'=DefaultNote]_s
\* A move changes no roster: the lines a line opened stay its own, because
\* coordination follows the tree, not the default branch.
MoveKeepsRoster == [][Move=>s.roster'=s.roster]_s
\* The root receives no note, whichever direction the role moves: it opens lines
\* and never ends whether or not it is the default branch. Because the note is a
\* keyed block and the root is never written one, the root's key holds the
\* roster alone, which is what a later compaction of the root replays.
RootGetsNoNote == IsRoot=>s.note=NoNote
\* A root that another line has taken the role from is a valid move target, so
\* the return to where the Session started is always admitted. `RootInit` starts
\* the Session on its root and `AnotherLineTakesRole` is the reachable state this
\* reads, so the root's own case is the one checked, not a parentless tool line.
ReturnToRootAdmitted == IsRoot /\ ~s.primary=>CanBeMovedTo
\* Admission in one order, first case: a line becomes the default branch only
\* while no ancestor's end is on its way down, so an end that already started
\* wins and the move is refused.
MoveNeedsOpenAncestry == [][s.primary' /\ ~s.primary => ~s.ancestorEnding]_s
\* Admission in one order, second case: a line starts ending only while no
\* descendant is the default branch, so a move that already happened wins and
\* the end is refused.
EndNeedsNoDefaultBelow == [][s.ending' /\ ~s.ending => ~s.descendantDefault]_s
\* A refused end writes nothing: `ending` moves only through an admitted end.
EndOnlyOnAllowedTarget == [][~CanBeEnded /\ ~CanEndSelf=>s.ending'=s.ending]_s
=============================================================================
