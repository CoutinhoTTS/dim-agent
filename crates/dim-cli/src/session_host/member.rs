//! One read-only member observation on the leader's existing connection.

use super::*;
use crate::model::view::MemberConversationState;

pub(super) enum Notice {
    Event(Event),
    Progress(Progress),
    State(bool),
}

enum Pending {
    Attach(u64),
    History(u64),
    Detach(u64),
}

pub(super) struct MemberObservation {
    projection_id: u64,
    session_id: String,
    projection: Projection,
    state: MemberConversationState,
    attached: bool,
    closing: bool,
    pending: Option<Pending>,
    after: u64,
    through: Option<u64>,
    cursor: u64,
    live: Vec<Notice>,
}

impl MemberObservation {
    pub(super) fn request_id(&self) -> Option<u64> {
        self.pending.as_ref().map(|pending| match pending {
            Pending::Attach(id) | Pending::History(id) | Pending::Detach(id) => *id,
        })
    }

    fn apply(&mut self, notice: Notice) {
        match notice {
            Notice::Event(event) if event.sequence > self.cursor => {
                self.cursor = event.sequence;
                self.projection.apply("main", &event);
            }
            Notice::Event(_) => {}
            Notice::Progress(progress) => self.projection.progress(&progress),
            Notice::State(running) => self.projection.set_running(running),
        }
    }
}

impl Worker {
    pub(super) fn watch_member(&mut self, member_id: Option<String>) {
        let desired = member_id.map(|id| (self.projection_id, id));
        if desired == self.desired_member {
            return;
        }
        self.desired_member = desired;
        if let Some(member) = &mut self.member {
            member.closing = true;
        }
    }

    pub(super) fn observes_member(&self, session_id: &str) -> bool {
        self.member
            .as_ref()
            .is_some_and(|member| member.session_id == session_id)
    }

    pub(super) async fn poll_member(&mut self) -> crate::Result<()> {
        loop {
            if self.member.is_none() {
                let Some((projection_id, session_id)) = self.desired_member.clone() else {
                    return Ok(());
                };
                self.member = Some(MemberObservation {
                    projection_id,
                    session_id,
                    projection: Projection::default(),
                    state: MemberConversationState::Loading,
                    attached: false,
                    closing: false,
                    pending: None,
                    after: 0,
                    through: None,
                    cursor: 0,
                    live: Vec::new(),
                });
                self.publish_member();
            }
            let member = self.member.as_ref().expect("selected observation");
            if member.pending.is_some() {
                return Ok(());
            }
            if member.closing && !member.attached {
                self.member = None;
                continue;
            }
            let (method, params) = if member.closing {
                ("session.detach", json!({"sessionId":member.session_id}))
            } else if !member.attached {
                ("session.attach", json!({"sessionId":member.session_id}))
            } else if member.state == MemberConversationState::Loading {
                let mut params = json!({
                    "sessionId":member.session_id,"branchId":"main",
                    "after":member.after,"limit":HISTORY_PAGE,
                });
                if let Some(through) = member.through {
                    params["through"] = json!(through);
                }
                ("session.history", params)
            } else {
                return Ok(());
            };
            let id = self.client.send(method, params).await?;
            self.member.as_mut().expect("admitted observation").pending = Some(match method {
                "session.attach" => Pending::Attach(id),
                "session.detach" => Pending::Detach(id),
                _ => Pending::History(id),
            });
            return Ok(());
        }
    }

    pub(super) fn member_reply(
        &mut self,
        result: Result<serde_json::Value, client::RpcError>,
    ) -> crate::Result<()> {
        let member = self.member.as_mut().expect("matched observation");
        let pending = member.pending.take().expect("matched member request");
        match pending {
            Pending::Attach(_) => match result {
                Ok(_) => member.attached = true,
                Err(error) if !member.closing => self.fail_member(error.to_string()),
                Err(_) => {}
            },
            Pending::Detach(_) => {
                result?;
                self.member = None;
            }
            Pending::History(_) if member.closing => {}
            Pending::History(_) => {
                let page = result
                    .map_err(|error| error.to_string())
                    .and_then(|value| History::decode(value, member.after, member.through));
                match page {
                    Err(error) => self.fail_member(error),
                    Ok(page) => {
                        member.after = page.next_after;
                        member.through = Some(page.through);
                        for event in page.events {
                            member.apply(Notice::Event(event));
                        }
                        member.cursor = page.next_after;
                        if page.done {
                            member.state = MemberConversationState::Live;
                            for notice in std::mem::take(&mut member.live) {
                                member.apply(notice);
                            }
                        }
                        self.changed();
                    }
                }
            }
        }
        Ok(())
    }

    pub(super) fn member_notice(&mut self, notice: Notice) {
        let member = self.member.as_mut().expect("routed observation");
        if member.closing {
            return;
        }
        if member.state == MemberConversationState::Loading {
            member.live.push(notice);
        } else {
            member.apply(notice);
        }
        self.changed();
    }

    pub(super) fn member_closed(&mut self) {
        let member = self.member.as_mut().expect("closed observation");
        if member.closing {
            return;
        }
        member.state = MemberConversationState::Closed;
        self.publish_member();
        self.member.as_mut().expect("closed observation").closing = true;
        self.desired_member = None;
    }

    fn fail_member(&mut self, error: String) {
        self.member.as_mut().expect("failed observation").state =
            MemberConversationState::ReadError(error);
        self.publish_member();
        self.member.as_mut().expect("failed observation").closing = true;
        self.desired_member = None;
    }

    pub(super) fn publish_member(&self) {
        if let Some(member) = &self.member
            && !member.closing
            && member.projection_id == self.projection_id
        {
            self.send(Update::MemberConversation {
                projection_id: member.projection_id,
                session_id: member.session_id.clone(),
                snapshot: Box::new(member.projection.snapshot()),
                state: member.state.clone(),
            });
        }
    }
}
