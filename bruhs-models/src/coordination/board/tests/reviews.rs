use super::*;

fn reviewed() -> (CoordinatedTask, CoordinatedTask) {
    let parent = task(Relationship::Root, TaskState::Completed, 1);
    let review = child(&parent, Relationship::Verify, TaskState::Completed, 2);
    (parent, review)
}

fn with_spec() -> (CoordinatedTask, CoordinatedTask, LivingSpec) {
    let (mut parent, mut review) = reviewed();
    let spec = LivingSpec {
        id: ThreadId::new_v4(),
        project_path: parent.project_path.clone(),
        title: "Acceptance criteria".into(),
        content: "The saved change passes its regression checks.".into(),
        revision: 1,
        updated_at: EpochMs::saturating_from_i64(1),
    };
    parent.spec_id = Some(spec.id.clone());
    parent.spec_revision = Some(spec.revision);
    review.spec_id = Some(spec.id.clone());
    review.spec_revision = Some(spec.revision);
    (parent, review, spec)
}

fn snapshot_cards(
    parent: CoordinatedTask,
    review: CoordinatedTask,
    specs: Vec<LivingSpec>,
) -> Vec<BoardCard> {
    cards(
        &CoordinationSnapshot {
            tasks: vec![parent, review],
            specs,
            ..CoordinationSnapshot::default()
        },
        &BTreeMap::new(),
    )
}

#[test]
fn a_completed_report_with_findings_requires_review_instead_of_claiming_success() {
    let (parent, mut review) = reviewed();
    review.messages.push(TaskMessage {
        role: "assistant".into(),
        text: "FAIL: the regression test panics. Do not accept this change.".into(),
        at: EpochMs::saturating_from_i64(3),
    });
    let review_id = review.id.clone();
    let cards = board(vec![parent, review], &[]);
    assert_eq!(only(&cards), (Lane::Validating, "Review ready"));
    assert_eq!(cards[0].status.tone, Tone::Action);
    assert!(!cards[0].status.busy);
    let report = cards[0].verification.as_ref().unwrap();
    assert_eq!(report.task_id, review_id);
    assert_eq!(report.freshness, ReviewFreshness::MatchesTask);
}

#[test]
fn verifier_process_states_describe_the_run_instead_of_test_outcomes() {
    let cases = [
        (TaskState::Completed, "Review ready", Tone::Action, false),
        (TaskState::Failed, "Verifier failed", Tone::Danger, false),
        (
            TaskState::Interrupted,
            "Review interrupted",
            Tone::Warning,
            false,
        ),
        (
            TaskState::Limited,
            "Verifier usage limit",
            Tone::Warning,
            false,
        ),
        (TaskState::Cancelled, "Review stopped", Tone::Neutral, false),
        (
            TaskState::WaitingApproval,
            "Verifier needs approval",
            Tone::Action,
            false,
        ),
        (TaskState::Ready, "Verifying", Tone::Info, true),
        (TaskState::Running, "Verifying", Tone::Info, true),
        (TaskState::WaitingChild, "Verifying", Tone::Info, true),
    ];
    for (state, label, tone, busy) in cases {
        let (parent, mut review) = reviewed();
        review.state = state;
        let cards = board(vec![parent, review], &[]);
        assert_eq!(only(&cards), (Lane::Validating, label), "{state:?}");
        assert_eq!((cards[0].status.tone, cards[0].status.busy), (tone, busy));
    }
}

#[test]
fn editing_the_saved_specification_invalidates_the_old_report_before_another_turn() {
    let (parent, review, mut spec) = with_spec();
    let cards = snapshot_cards(parent.clone(), review.clone(), vec![spec.clone()]);
    assert_eq!(only(&cards), (Lane::Validating, "Review ready"));
    spec.revision = 2;
    let cards = snapshot_cards(parent, review, vec![spec]);
    assert_eq!(only(&cards), (Lane::Building, "Review outdated"));
    let report = cards[0].verification.as_ref().unwrap();
    assert_eq!(report.spec_revision, Some(1));
    assert_eq!(report.freshness, ReviewFreshness::SpecificationChanged);
    assert_eq!(cards[0].verifications, 1);
}

#[test]
fn attaching_a_different_specification_does_not_reuse_a_report_at_the_same_revision() {
    let (mut parent, review, mut spec) = with_spec();
    spec.id = ThreadId::new_v4();
    parent.spec_id = Some(spec.id.clone());
    let cards = snapshot_cards(parent, review, vec![spec]);
    assert_eq!(only(&cards), (Lane::Building, "Review outdated"));
    assert_eq!(
        cards[0].verification.as_ref().unwrap().freshness,
        ReviewFreshness::SpecificationChanged
    );
}

#[test]
fn an_unavailable_specification_cannot_keep_a_report_current() {
    let (parent, review, _) = with_spec();
    let cards = snapshot_cards(parent, review, vec![]);
    assert_eq!(only(&cards), (Lane::Building, "Specification unavailable"));
    assert_eq!(
        cards[0].verification.as_ref().unwrap().freshness,
        ReviewFreshness::SpecificationUnavailable
    );
}

#[test]
fn a_new_request_invalidates_a_report_even_in_the_same_millisecond() {
    let (mut parent, review) = reviewed();
    parent.messages.push(TaskMessage {
        role: "user".into(),
        text: "Fix the remaining failing case.".into(),
        at: review.created_at,
    });
    let cards = board(vec![parent, review], &[]);
    assert_eq!(only(&cards), (Lane::Building, "Review outdated"));
    assert_eq!(
        cards[0].verification.as_ref().unwrap().freshness,
        ReviewFreshness::TaskChanged
    );
}

#[test]
fn bringing_back_a_report_is_not_a_new_request() {
    let (mut parent, review) = reviewed();
    parent.messages.push(TaskMessage {
        role: "assistant".into(),
        text: "Verification report: one failing regression test.".into(),
        at: EpochMs::saturating_from_i64(3),
    });
    assert_eq!(
        only(&board(vec![parent, review], &[])),
        (Lane::Validating, "Review ready")
    );
}

#[test]
fn reviews_from_a_different_workspace_or_unknown_capture_remain_inspectable() {
    let (parent, mut review) = reviewed();
    review.working_path = "/another-worktree".into();
    let cards = board(vec![parent.clone(), review.clone()], &[]);
    assert_eq!(only(&cards), (Lane::Building, "Review outdated"));
    assert_eq!(
        cards[0].verification.as_ref().unwrap().freshness,
        ReviewFreshness::WorkspaceChanged
    );
    review.working_path.clone_from(&parent.working_path);
    review.source_revision = None;
    let cards = board(vec![parent, review], &[]);
    assert_eq!(only(&cards), (Lane::Building, "Review context unavailable"));
    assert_eq!(
        cards[0].verification.as_ref().unwrap().freshness,
        ReviewFreshness::ContextUnavailable
    );
}

#[test]
fn a_fresh_report_replaces_the_outdated_summary_and_preserves_history() {
    let (mut parent, older) = reviewed();
    parent.messages.push(TaskMessage {
        role: "user".into(),
        text: "Address the finding.".into(),
        at: EpochMs::saturating_from_i64(3),
    });
    let newest = child(&parent, Relationship::Verify, TaskState::Completed, 4);
    let newest_id = newest.id.clone();
    let cards = board(vec![parent, older, newest], &[]);
    assert_eq!(only(&cards), (Lane::Validating, "Review ready"));
    assert_eq!(cards[0].verification.as_ref().unwrap().task_id, newest_id);
    assert_eq!(cards[0].verifications, 2);
}

#[test]
fn forge_checks_and_active_parent_work_keep_their_own_status() {
    let (parent, review) = reviewed();
    let failing = pull(PullState::Open, ChecksState::Failing, None, "MERGEABLE");
    let cards = board(vec![parent.clone(), review.clone()], &[(0, vec![failing])]);
    assert_eq!(only(&cards), (Lane::Validating, "Checks failing"));
    assert!(cards[0].verification.is_some());
    let mut working = parent;
    working.state = TaskState::Running;
    assert_eq!(
        only(&board(vec![working, review], &[])),
        (Lane::Validating, "Working")
    );
}

#[test]
fn cards_from_an_older_host_deserialize_without_review_metadata() {
    let (parent, _) = reviewed();
    let original = board(vec![parent], &[]).remove(0);
    let mut value = serde_json::to_value(&original).unwrap();
    value.as_object_mut().unwrap().remove("verification");
    let decoded: BoardCard = serde_json::from_value(value).unwrap();
    assert_eq!(decoded, original);
}
