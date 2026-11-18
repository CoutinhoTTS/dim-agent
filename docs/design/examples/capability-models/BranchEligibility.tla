-------------------- MODULE BranchEligibility --------------------
EXTENDS Naturals, Sequences, FiniteSets, TLC
\* One line's eligibility, in a Session whose default branch is a position in
\* the tree. Nothing here is a role marker: a line is the root when it has no
\* parent, and its opener is that parent, so both are read from the tree
\* (`IsRoot`, `HasOpener`). `primary` says the line is the Session's default
\* branch: the line its conversation is on. `source` says where the line was
\* created from, and only that.
\*
\* This model checks what a line may do, never when it should do it: opening
\* policy is not modelled. What it does fix is who may open a line. Two lines
\* qualify, the root always and the current default branch while it holds the
\* role (`CanOpen`), so at most two lines in a Session can open one. That is
\* what keeps the root able to answer a user who asks it for a line while
\* another line holds the role: a guard that let only the default branch open
\* one would refuse that request.
\*
\* `ancestorEnding` and `descendantDefault` are the tree facts the admission
\* rules read: some ancestor's end is already on its way down, which the
\* cascade will complete, and some descendant is the default branch. The
\* environment sets each of them, and that is how the two conflicting orderings
\* reach this line: an ancestor's end can commit first, or a descendant can
\* become the default branch first. Exactly one of the two is admitted.
\*
\* `openSlots` is the Session's room for one more line that the tool opened and
\* that counts: the cap counts open tool lines except those on the protected
\* path, which the default branch and its ancestors are. So this line's own
\* opening a line consumes a slot only when the line would count, and becoming
\* the default branch, or being on that path, frees no slot and closes nothing;
\* it only changes what a later open would cost.
VARIABLE s
NoParent == "none"
Cap == 2
\* A line the user may choose as the default branch. A plugin line follows its
\* creator's protocol, and a line a client created before the upgrade keeps its
\* stored `host` row, so neither is selectable.
Selectable == {"tool","user"}
IsRoot == s.parent = NoParent
HasOpener == ~IsRoot
\* The Session's two openers, and the whole rule: the root in every state, and
\* the default branch while it is one. A plugin line is never either of them,
\* because it is never the default branch and is never the root.
CanOpen == IsRoot \/ (s.primary /\ s.source \in Selectable)
CanBeMovedTo == ~s.primary /\ s.source\in Selectable /\ ~s.ended /\ ~s.ending /\ ~s.ancestorEnding
\* Only a tool line ends itself: a line the user opened is ended by the user, a
\* plugin line by its creator's protocol, and the root never ends. A line that
\* is the default branch, or that has a descendant which is, is refused too,
\* because ending a line ends everything below it.
CanEndSelf == HasOpener /\ ~s.primary /\ ~s.descendantDefault /\ ~s.ancestorEnding /\ s.source="tool"
\* An end asked for from outside the line: the user's, a creator's or a
\* cascade's. The end entry checks the target only, whoever asks.
CanBeEnded == HasOpener /\ ~s.primary /\ ~s.descendantDefault /\ ~s.ended /\ ~s.ending
\* An open from this line: it must be allowed to open one, the Session must have
\* a slot left, and the line is not ending. The caller's own state decides what
\* the open costs, because a line opened by a caller on the protected path sits
\* on it too: that path is the default branch and every ancestor of it, and it is
\* exactly what the cap does not count. `children` is bounded at two so the model
\* stays finite; nothing in the rules depends on that number.
CanCreate == CanOpen /\ ~s.ending /\ ~s.ended /\ s.openSlots>0 /\ Cardinality(s.children)<2
Init == \E source\in{"tool","user","plugin","hostOld"},parent\in{NoParent,"P1"},
   primary\in BOOLEAN,notice\in BOOLEAN,openSlots\in 0..Cap:
 /\ (source="plugin" => ~primary)
 /\ (source="hostOld" => ~primary)
 /\ s=[parent|->parent,source|->source,primary|->primary,notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
     ended|->FALSE,ending|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,notes|->0,
     blocks|-><<>>,roster|->{1,2},endedNotice|->{},externalEnds|->{},
     ancestorEnding|->FALSE,descendantDefault|->FALSE,openSlots|->openSlots,children|->{}]
\* The root is not the default branch in every state: a move to another line
\* leaves it open and no longer the default branch, which is the state in which
\* it is a valid target again.
RootInit == \E primary\in BOOLEAN,notice\in BOOLEAN,openSlots\in 0..Cap:
 /\ s=[parent|->NoParent,source|->"root",primary|->primary,notice|->notice,head|->1,opening|->1,cut|->0,cursor|->0,reported|->1,held|->FALSE,
     ended|->FALSE,ending|->FALSE,turn|->FALSE,steered|->FALSE,ownEnd|->FALSE,pending|->FALSE,recovery|->FALSE,idle|->0,notes|->0,
     blocks|-><<>>,roster|->{1,2},endedNotice|->{},externalEnds|->{},
     ancestorEnding|->FALSE,descendantDefault|->FALSE,openSlots|->openSlots,children|->{}]
\* Becoming the default branch. A target whose ancestor is already ending is
\* refused, because the cascade will reach it, and so is the line that already
\* holds the position, whatever line it is: the root is refused while it is the
\* default branch and accepted once another line holds the role. A move closes
\* no line, so the Session's room changes only because what counts changed.
Move == /\ CanBeMovedTo /\ s'=[s EXCEPT !.primary=TRUE, !.notes=@+1]
\* Opening a line. The opener's own slot accounting is the Session's: a line it
\* opens while it is on the protected path is on that path too, so that open
\* costs nothing, and one it opens otherwise costs a slot.
Create == /\ CanCreate
 /\ LET costs == IF s.primary \/ s.descendantDefault THEN FALSE ELSE TRUE
    IN s'=[s EXCEPT !.children=@\cup{Cardinality(s.children)+1},
                       !.openSlots=IF costs THEN @-1 ELSE @]
\* The two tree facts. Each can arrive at any time, and only the admission
\* rules decide what follows from it.
BeginAncestorEnd == /\ ~s.ancestorEnding /\ ~s.ended /\ s'=[s EXCEPT !.ancestorEnding=TRUE]
BeginDescendantDefault == /\ ~s.descendantDefault /\ ~s.primary /\ ~s.ended
 /\ s'=[s EXCEPT !.descendantDefault=TRUE]
ParentAdvance == /\ s.head<2 /\ s'=[s EXCEPT !.head=@+1]
Read(after,through,ok) == /\ after<=s.head /\ through<=s.head /\ through>=after
 /\ LET floor==IF IsRoot THEN after ELSE IF after<s.cut THEN s.cut ELSE after
 IN /\ through>=floor /\ s'=[s EXCEPT !.cursor=IF ok THEN through ELSE @]
Block == /\ HasOpener /\ ~s.ended /\ ~s.held /\ Len(s.blocks)<1
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
Next == Move\/Create\/BeginAncestorEnd\/BeginDescendantDefault\/ParentAdvance
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
\* The default branch is an open conversation line.
DefaultIsOpenConversationLine == s.primary=>((IsRoot \/ s.source\in Selectable) /\ ~s.ended /\ ~s.ending)
\* Who may open a line: the root always, and the default branch while it holds
\* the role. Every other line, a former default branch included, is refused, so
\* at most two lines in a Session qualify and the root never loses the right.
OnlyRootAndDefaultOpen == [][Create=>s.primary \/ IsRoot]_s
RootAlwaysCanOpen == [][IsRoot=>CanOpen]_s
\* The cap. A line that counts is a line the tool opened, is still open, and is
\* not on the protected path; an open is admitted only while a slot remains, and
\* a move or a switch takes no slot and closes nothing.
\* The cap only ever refuses a new open: an admitted open has room by
\* `CanCreate`, and an open that costs a slot spends exactly one.
CapOnlyRefusesNewOpens == [][Create=>s.openSlots>0 /\ s.openSlots'<=s.openSlots]_s
MoveTakesNoSlot == [][Move=>s.openSlots'=s.openSlots]_s
SlotsStayWithinCap == s.openSlots\in 0..Cap
\* A switch never closes a line: the roster and the lines are untouched by it.
MoveClosesNothing == [][Move=>s.roster'=s.roster /\ s.ended'=s.ended]_s
\* A line that opens nothing is not on the protected path, so its open costs a
\* slot: the counting rule is what keeps a user who keeps continuing on tool
\* children from filling the cap with the line they are working on.
OpenOffPathSpendsSlot == [][Create /\ ~s.primary /\ ~s.descendantDefault=>s.openSlots'=s.openSlots-1]_s
\* Admission in one order, first case: a line becomes the default branch only
\* while no ancestor's end is on its way down, so an end that already started
\* wins and the move is refused.
MoveNeedsOpenAncestry == [][s.primary' /\ ~s.primary=>~s.ancestorEnding]_s
\* Admission in one order, second case: a line starts ending only while no
\* descendant is the default branch, so a move that already happened wins and
\* the end is refused.
EndNeedsNoDefaultBelow == [][s.ending' /\ ~s.ending=>~s.descendantDefault]_s
=============================================================================
