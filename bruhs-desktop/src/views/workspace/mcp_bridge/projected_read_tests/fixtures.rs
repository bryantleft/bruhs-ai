//! Synthetic owned records and distinct GPUI chat entities, outside all timers.
use super::{record_id, Workspace, DOMAINS, HIDDEN};
use bruhs_models::*;
use gpui::{Context, Window};

pub(super) fn populate(
    workspace: &mut Workspace,
    group: &str,
    records: usize,
    window: &mut Window,
    cx: &mut Context<'_, Workspace>,
) {
    workspace.life_context = LifeContext::default();
    workspace.meeting_notes.clear();
    workspace.meetings_hub.update(cx, |hub, _| {
        hub.meetings.clear();
        hub.people.clear();
    });
    for domain in DOMAINS {
        workspace
            .automation_state
            .permissions
            .set(domain, AgentAccess::Read, true);
    }
    let label = |index| format!("ΟΣ Café İ K metadata {index}");
    if group == "life" {
        let finance = &mut workspace.life_context.finance;
        finance.privacy.allow_agent_context = true;
        let health = &mut workspace.life_context.health;
        health.privacy.allow_agent_context = true;
        for index in 0..records {
            let now = i64::try_from(index).expect("fixture timestamp fits i64");
            let id = record_id(index, records);
            let mut account = FinanceAccount::manual(label(index), now, now);
            account.id.clone_from(&id);
            finance.accounts.push(account);
            let mut transaction = FinanceTransaction::manual(label(index), "Category", now, now);
            transaction.id.clone_from(&id);
            if index == 0 {
                transaction.note = Some(HIDDEN.repeat(256));
            }
            finance.transactions.push(transaction.into());
            let mut budget = FinanceBudget::monthly(label(index), now);
            budget.id.clone_from(&id);
            finance.budgets.push(budget);
            let mut income = IncomeStream::monthly(label(index), now);
            income.id.clone_from(&id);
            finance.income_streams.push(income);
            let mut holding = Holding::manual("META", now, now);
            holding.id.clone_from(&id);
            holding.name = label(index);
            holding.quantity = 0.125;
            holding.cost_basis_minor = Some(now);
            finance.holdings.push(holding);
            let mut workout = WorkoutLog::manual(label(index), 30, now);
            workout.id.clone_from(&id);
            health.workouts.push(workout);
            let mut sleep = SleepLog::manual(480, 90, now);
            sleep.id.clone_from(&id);
            health.sleep.push(sleep);
            let mut recovery = RecoveryLog::manual(85, now);
            recovery.id.clone_from(&id);
            if index == 0 {
                recovery.note = Some(HIDDEN.repeat(256));
            }
            health.recovery.push(recovery);
            let mut habit = Habit::weekly(label(index), 5);
            habit.id = id;
            health.habits.push(habit);
        }
    } else if group == "summaries" {
        workspace.meetings_hub.update(cx, |hub, _| {
            for index in 0..records {
                let now = i64::try_from(index).expect("fixture timestamp fits i64");
                let id = record_id(index, records);
                let mut meeting = Meeting::new(
                    label(index),
                    f64::from(u32::try_from(index).expect("fixture timestamp fits u32")),
                );
                meeting.id.clone_from(&id);
                if index == 0 {
                    meeting.audio_path = Some(format!("/synthetic/{HIDDEN}.wav"));
                    meeting.summary = Some(MeetingSummary {
                        tldr: HIDDEN.repeat(256),
                        action_items: vec![],
                        decisions: vec![],
                        topics: vec![],
                        generated_by: "fixture".into(),
                        generated_at: 1,
                    });
                }
                hub.meetings.push(meeting);
                let mut person = PersonProfile::new(label(index));
                person.id = id;
                person.aliases = vec![format!("Alias {index}")];
                person.created_at = now;
                person.updated_at = now;
                if index == 0 {
                    person.voice_embedding = Some(vec![0.125; 512]);
                    person.observations.push(PersonObservation {
                        at_meeting_id: "fixture".into(),
                        kind: ObservationKind::Quote,
                        text: HIDDEN.repeat(256),
                        created_at: now,
                    });
                }
                hub.people.push(person.into());
            }
        });
        workspace.meeting_notes = (0..records)
            .map(|index| MeetingNote {
                id: record_id(index, records),
                title: label(index),
                body: if index == 0 {
                    HIDDEN.repeat(256)
                } else {
                    "Saved local body".into()
                },
                created_at: i64::try_from(index).expect("fixture timestamp fits i64"),
                updated_at: i64::try_from(index).expect("fixture timestamp fits i64"),
                recording_id: None,
            })
            .collect();
    } else if group == "chat" {
        workspace.threads.clear();
        for index in 0..records {
            let mut thread = Workspace::create_thread_with_title(
                "Fixture project".into(),
                String::new(),
                "fixture-model".into(),
                workspace.permission_mode,
                &label(index),
                window,
                cx,
            );
            thread.id = record_id(index, records);
            thread.updated_at_ms =
                EpochMs::new(i64::try_from(index).expect("fixture timestamp fits i64"))
                    .expect("valid fixture timestamp");
            thread.chat.update(cx, |chat, _| {
                chat.thread_id = Some(thread.id.clone());
                if index == 0 {
                    chat.messages = vec![crate::views::chat::Message::user(HIDDEN.repeat(256))];
                }
            });
            workspace.threads.push(thread);
        }
    } else {
        unreachable!()
    }
}
