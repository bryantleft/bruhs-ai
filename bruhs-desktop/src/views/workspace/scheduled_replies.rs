//! Deliver scheduled turns into the original chat. Save the cursor with the
//! transcript so a host restart or a replay cannot duplicate a reply.
use super::*;
use crate::app_state::spawn_tokio;
use crate::views::chat::{Message, TurnState};
use crate::views::reactive_automations::DesktopTaskActivity;
use bruhs_models::reactive::{DesktopTaskTurn, RunStatus};

impl Workspace {
    pub(super) fn read_scheduled_replies(
        &mut self,
        activity: &DesktopTaskActivity,
        cx: &mut Context<'_, Self>,
    ) {
        // Keep the confirmation accurate after a restart while the host is offline.
        let links = activity
            .links
            .iter()
            .filter(|(id, _)| {
                self.threads.iter().any(|thread| &thread.id == *id)
                    || self.archived_threads.iter().any(|thread| thread.id() == id.as_str())
            })
            .map(|(id, tasks)| (id.clone(), tasks.clone()))
            .collect();
        if self.desktop_task_links != links {
            self.desktop_task_links = links;
            self.persist(cx);
        }
        for (id, latest) in &activity.threads {
            let Some(thread) = self.threads.iter().find(|t| &t.id == id) else {
                continue;
            };
            let key = format!("{}/{}", activity.store, id);
            let after = self
                .desktop_task_cursors
                .get(&key)
                .copied()
                .unwrap_or_default();
            if after >= *latest
                || thread.chat.read(cx).is_streaming
                || !self.reading_desktop_tasks.insert(key.clone())
            {
                continue;
            }
            let client = meetings_client();
            let id = id.clone();
            let latest = *latest;
            cx.spawn(async move |workspace, cx| {
                let mut cursor = after;
                while cursor < latest {
                    let target = id.clone();
                    let client = client.clone();
                    let response = cx
                        .update(|cx| {
                            spawn_tokio(cx, async move {
                                client.desktop_task_turns(&target, cursor).await
                            })
                        })
                        .await;
                    let Ok(turns) = response else { break };
                    if turns.is_empty() {
                        break;
                    }
                    let Some(applied) = workspace
                        .update(cx, |ws, cx| {
                            ws.apply_scheduled_replies(&id, &key, turns, cx)
                        })
                        .ok()
                        .flatten()
                    else {
                        break;
                    };
                    if applied <= cursor {
                        break;
                    }
                    cursor = applied;
                }
                let _ = workspace.update(cx, |ws, _| {
                    ws.reading_desktop_tasks.remove(&key);
                });
            })
            .detach();
        }
    }

    fn apply_scheduled_replies(
        &mut self,
        id: &str,
        key: &str,
        turns: Vec<DesktopTaskTurn>,
        cx: &mut Context<'_, Self>,
    ) -> Option<i64> {
        let thread = self.threads.iter_mut().find(|t| t.id == id)?;
        if thread.chat.read(cx).is_streaming {
            return None;
        }
        let mut cursor = self
            .desktop_task_cursors
            .get(key)
            .copied()
            .unwrap_or_default();
        let previous = cursor;
        thread.chat.update(cx, |chat, cx| {
            for turn in turns {
                if turn.sequence <= cursor || turn.run.thread_id.as_str() != id {
                    continue;
                }
                let run = turn.run;
                let started = run.created_at.as_millis();
                let ended = run.updated_at.as_millis();
                let prompt = run.owner_message.as_deref().unwrap_or(&run.prompt);
                let mut request = Message::user(format!("{}\n\n{prompt}", turn.title));
                request.started_at_ms = Some(started);
                let mut answer = Message::assistant_streaming(started, Some(turn.model.clone()));
                answer.content = run.result.unwrap_or_default();
                answer.completed_at_ms = Some(ended);
                answer.state = if run.status == RunStatus::Failed {
                    TurnState::Failed {
                        error: run
                            .error
                            .unwrap_or_else(|| "The scheduled task failed.".into()),
                    }
                } else {
                    TurnState::Finished
                };
                // A later interactive turn already owns the current session.
                let newer = chat
                    .messages
                    .iter()
                    .rev()
                    .find_map(|m| m.completed_at_ms)
                    .is_some_and(|time| time > ended);
                chat.insert_scheduled_exchange(request, answer);
                if !newer && chat.model_id == turn.model && run.status == RunStatus::Completed {
                    if let Some(session) = run.session_id {
                        chat.session_id = Some(session);
                        chat.session_provider = Some(AgentProvider::for_model(&turn.model));
                        chat.fresh_session = false;
                        chat.import_context = None;
                    } else {
                        chat.session_id = None;
                        chat.session_provider = None;
                        chat.fresh_session = true;
                        chat.import_context = chat.conversation_context(&chat.messages);
                    }
                }
                thread.updated_at_ms = thread.updated_at_ms.max(run.updated_at);
                cursor = turn.sequence;
            }
            if cursor != previous {
                cx.notify();
            }
        });
        if cursor == previous {
            return Some(cursor);
        }
        self.desktop_task_cursors.insert(key.to_owned(), cursor);
        if self.active_thread_id.as_deref() != Some(id) {
            self.unread_thread_ids.insert(id.to_owned());
        }
        self.persist_and_notify(cx);
        Some(cursor)
    }
}

#[cfg(test)]
mod tests {
    use super::{DesktopTaskTurn, Message, TurnState};
    use crate::views::workspace::tests::thread_menu_workspace;

    #[gpui::test]
    fn saved_task_links_hold_the_chat_until_the_host_acknowledges_deletion(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ws, cx, id, _) = thread_menu_workspace(cx);
        ws.update_in(cx,|ws,window,cx| {
            ws.confirm_thread_delete=false;
            ws.desktop_task_links.insert(id.clone(),[("weekly".into(),"Weekly review".into())].into());
            ws.request_thread_delete(&id,window,cx);
            assert!(ws.threads.iter().any(|thread|thread.id==id));
            let panel=ws.reactive_automations.read(cx);
            assert!(matches!(panel.sent_for_test().last(), Some(bruhs_models::reactive::ReactiveCommand::DeleteTaskThread {thread_id,linked_tasks})
                if thread_id.as_str()==id && linked_tasks.contains("weekly")));
            let snapshot=ws.snapshot(cx);
            assert_eq!(snapshot.desktop_task_links[&id]["weekly"],"Weekly review");
        });
    }

    #[gpui::test]
    fn scheduled_reply_replay_preserves_order_and_the_latest_interactive_session(
        cx: &mut gpui::TestAppContext,
    ) {
        let (ws, cx, id, _) = thread_menu_workspace(cx);
        ws.update_in(cx,|ws,_,cx| {
            let chat=ws.threads.iter().find(|t|t.id==id).unwrap().chat.clone();
            let model=chat.read(cx).model_id.clone();
            chat.update(cx,|chat,_| {
                let mut answer=Message::assistant_streaming(300,Some(model.clone()));
                answer.content="Later answer".into();
                answer.completed_at_ms=Some(400);
                answer.state=TurnState::Finished;
                chat.messages=vec![Message::user("Later question"),answer];
                chat.session_id=Some("later-session".into());
            });
            let target=|seq:i64,time:i64| serde_json::from_value::<DesktopTaskTurn>(serde_json::json!({
                "sequence":seq,"title":"Build review","model":model,
                "run":{"id":format!("run-{seq}"),"agent_id":id,"status":"completed","prompt":"Check build",
                       "thread_id":id,"created_at":time,"updated_at":time+10,"not_before":time,
                       "result":"Build passed","session_id": if seq==1 {Some("earlier-session")} else {None}}
            })).unwrap();
            let key=format!("fixture/{id}");
            for _ in 0..2 { assert_eq!(ws.apply_scheduled_replies(&id,&key,vec![target(1,100)],cx),Some(1)); }
            let c=chat.read(cx);
            assert_eq!(c.messages.iter().map(|m|m.content.as_str()).collect::<Vec<_>>(),
                       ["Build review\n\nCheck build","Build passed","Later question","Later answer"]);
            assert_eq!(c.session_id.as_deref(),Some("later-session"));
            chat.update(cx,|c,_|c.is_streaming=true);
            assert_eq!(ws.apply_scheduled_replies(&id,&key,vec![target(2,500)],cx),None);
            chat.update(cx,|c,_|c.is_streaming=false);
            assert_eq!(ws.apply_scheduled_replies(&id,&key,vec![target(2,500)],cx),Some(2));
            let c=chat.read(cx);
            assert!(c.session_id.is_none());
            assert!(c.import_context.as_deref().is_some_and(|context| context.contains("Later question") && context.contains("Build passed")));
            let snapshot=ws.snapshot(cx);
            assert_eq!(snapshot.desktop_task_cursors[&key],2);
            assert_eq!(snapshot.threads.iter().find(|t|t.id==id).unwrap().messages.len(),6);
        });
    }
}
