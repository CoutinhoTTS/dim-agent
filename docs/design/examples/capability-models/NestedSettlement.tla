-------------------- MODULE NestedSettlement --------------------
EXTENDS Naturals, Sequences, FiniteSets, TLC
VARIABLE s
Children == 1..2
Init == s=[accepted |-> {}, effects |-> {}, childDone |-> {}, parentDone |-> FALSE,
 parentEffects |-> {}, cancelled |-> FALSE, expired |-> FALSE]
Child(i) == /\ ~s.parentDone /\ ~s.cancelled /\ i\notin s.accepted
 /\ s'=[s EXCEPT !.accepted=@\cup{i}]
Effect(i) == /\ i\in s.accepted\s.childDone /\ ~s.parentDone
 /\ s'=[s EXCEPT !.effects=@\cup{i}]
ChildResult(i) == /\ i\in s.accepted\s.childDone /\ ~s.parentDone
 /\ s'=[s EXCEPT !.childDone=@\cup{i}]
Cancel == s'=[s EXCEPT !.cancelled=TRUE]
Expire == /\ s.cancelled /\ s'=[s EXCEPT !.expired=TRUE]
ParentResult == /\ ~s.parentDone /\ (s.accepted=s.childDone \/ s.expired)
 /\ s'=[s EXCEPT !.parentDone=TRUE, !.parentEffects=s.effects\cap s.childDone]
Next == (\E i\in Children:Child(i)\/Effect(i)\/ChildResult(i))\/Cancel\/Expire\/ParentResult
Spec == Init /\ [][Next]_s
ConfirmedEffectsRetained == s.parentDone => s.parentEffects=s.effects\cap s.childDone
NoUnconfirmedEffects == s.parentEffects\subseteq s.childDone
ChildBeforeParentUnlessDeadline == s.parentDone /\ ~s.expired => s.accepted=s.childDone
CancellationIsNotRollback == [][Cancel => s.effects'=s.effects]_s
=============================================================================
