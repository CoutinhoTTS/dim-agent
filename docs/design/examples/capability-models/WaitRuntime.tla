-------------------- MODULE WaitRuntime --------------------
EXTENDS Naturals, Sequences, FiniteSets, TLC
VARIABLE s
Init == s=[open |-> FALSE, arrivals |-> {}, seen |-> {}, taken |-> {}, waiting |-> 0,
 closing |-> FALSE, tracked |-> {}, finished |-> {}, generation |-> 0, accepted |-> 0,
 boundaries |-> <<>>, recovered |-> FALSE, target |-> 0, selections |-> <<>>]
Start == /\ ~s.closing /\ Len(s.boundaries)<3 /\ s'=[s EXCEPT !.open=TRUE, !.boundaries=Append(@,TRUE), !.recovered=FALSE]
End == /\ ~s.closing /\ Len(s.boundaries)<3 /\ s'=[s EXCEPT !.open=FALSE, !.arrivals={}, !.boundaries=Append(@,FALSE), !.recovered=FALSE]
Arrive(i) == /\ s.open /\ ~s.closing /\ i\notin s.seen /\ s'=[s EXCEPT !.arrivals=@\cup{i}, !.seen=@\cup{i}, !.recovered=FALSE]
Admit(g,t) == /\ g=s.generation /\ s.accepted<3 /\ s.waiting=0 /\ ~s.closing
 /\ s'=[s EXCEPT !.waiting=s.accepted+1, !.tracked=@\cup{s.accepted+1}, !.accepted=@+1, !.target=t]
Take(i) == /\ s.waiting#0 /\ ~s.closing /\ i\in s.arrivals /\ s.target\in{0,i}
 /\ s'=[s EXCEPT !.arrivals=@\{i}, !.taken=@\cup{i}, !.waiting=0, !.finished=@\cup{s.waiting},
 !.selections=Append(@,[target|->s.target,task|->i])]
EndCall == /\ s.waiting#0 /\ s'=[s EXCEPT !.waiting=0, !.finished=@\cup{s.waiting}]
ReleaseTracking(i) == /\ i\in s.tracked\cap s.finished /\ s'=[s EXCEPT !.tracked=@\{i}]
Close == /\ ~s.closing /\ s'=[s EXCEPT !.closing=TRUE]
Restore == /\ s.closing /\ s.tracked={} /\ s.generation<1
 /\ s'=[s EXCEPT !.closing=FALSE, !.generation=@+1, !.arrivals={}, !.recovered=TRUE,
 !.open=IF Len(s.boundaries)=0 THEN FALSE ELSE s.boundaries[Len(s.boundaries)]]
Next == Start\/End\/(\E i\in 1..2:Arrive(i)\/Take(i))\/(\E g\in 0..1,t\in 0..2:Admit(g,t))
 \/EndCall\/(\E i\in 1..3:ReleaseTracking(i))\/Close\/Restore
ShutdownCall == s.closing /\ EndCall
Spec == Init /\ [][Next]_s /\ WF_s(ShutdownCall) /\ (\A i\in 1..3:WF_s(ReleaseTracking(i)))
SlotTracked == s.waiting#0 => s.waiting\in s.tracked
TakenAbsent == s.arrivals\cap s.taken={}
ClosedTurnEmpty == ~s.open => s.arrivals={}
RestoredNoNotifications == s.recovered => s.arrivals={}
SelectionMatches == \A i\in 1..Len(s.selections):s.selections[i].target\in{0,s.selections[i].task}
NoAdmissionAfterClose == [][s.closing /\ s.generation'=s.generation => s.accepted'=s.accepted]_s
DrainBeforeRestore == [][s.generation'>s.generation => s.tracked={} /\ s.waiting=0]_s
OldHandleRejected == [][\A g\in 0..1,t\in 0..2:g<s.generation=>~Admit(g,t)]_s
CloseDrains == s.closing ~> s.tracked={}
=============================================================================
