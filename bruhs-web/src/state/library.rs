//! Captures, knowledge, inbox, relationships, life context and automations.

use super::*;

#[derive(Copy, Clone)]
pub struct CaptureOpen(pub ReadSignal<bool>);
#[derive(Copy, Clone)]
pub struct SetCaptureOpen(pub WriteSignal<bool>);
#[derive(Copy, Clone)]
pub struct CaptureEditId(pub ReadSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct SetCaptureEditId(pub WriteSignal<Option<String>>);
#[derive(Copy, Clone)]
pub struct CapturesSignal(pub ReadSignal<Vec<CaptureRecord>>);
#[derive(Copy, Clone)]
pub struct SetCapturesSignal(pub WriteSignal<Vec<CaptureRecord>>);

#[derive(Copy, Clone)]
pub struct KnowledgeDocumentsSignal(pub ReadSignal<Vec<KnowledgeDocument>>);
#[derive(Copy, Clone)]
pub struct SetKnowledgeDocumentsSignal(pub WriteSignal<Vec<KnowledgeDocument>>);
#[derive(Copy, Clone)]
pub struct SourceClipsSignal(pub ReadSignal<Vec<SourceClip>>);
#[derive(Copy, Clone)]
pub struct SetSourceClipsSignal(pub WriteSignal<Vec<SourceClip>>);

#[derive(Copy, Clone)]
pub struct AutomationStateSignal(pub DurableMetadata<AutomationState>);
#[derive(Copy, Clone)]
pub struct LifeSectionSignal(pub RwSignal<LifeSection>);

/// Captures, knowledge documents and sources, inbox conversations,
/// relationships, life context and automation state.
pub(super) fn provide() {
    let (capture_open, set_capture_open) = signal(false);
    let (capture_edit_id, set_capture_edit_id) = signal::<Option<String>>(None);
    let (captures, set_captures) = signal(Vec::<CaptureRecord>::new());
    let (knowledge_documents, set_knowledge_documents) = signal(Vec::<KnowledgeDocument>::new());
    let (source_clips, set_source_clips) = signal(Vec::<SourceClip>::new());
    let (inbox_conversations, set_inbox_conversations) = signal(Vec::<InboxConversation>::new());
    let (relationships, set_relationships) = signal(Vec::<RelationshipProfile>::new());

    provide_context(CaptureOpen(capture_open));
    provide_context(SetCaptureOpen(set_capture_open));
    provide_context(CaptureEditId(capture_edit_id));
    provide_context(SetCaptureEditId(set_capture_edit_id));
    provide_context(CapturesSignal(captures));
    provide_context(SetCapturesSignal(set_captures));
    provide_context(KnowledgeDocumentsSignal(knowledge_documents));
    provide_context(SetKnowledgeDocumentsSignal(set_knowledge_documents));
    provide_context(SourceClipsSignal(source_clips));
    provide_context(SetSourceClipsSignal(set_source_clips));
    provide_context(InboxConversationsSignal(inbox_conversations));
    provide_context(SetInboxConversationsSignal(set_inbox_conversations));
    provide_context(RelationshipsSignal(relationships));
    provide_context(SetRelationshipsSignal(set_relationships));
    provide_context(LifeContextSignal(DurableMetadata::<LifeContext>::new(
        "bruhs.lifeContext",
    )));
    provide_context(LifeSectionSignal(RwSignal::new(LifeSection::Finance)));
    provide_context(SelectedLifeRecordSignal(RwSignal::new(None)));
    provide_context(AutomationStateSignal(
        DurableMetadata::<AutomationState>::new("bruhs.automationState"),
    ));
}
