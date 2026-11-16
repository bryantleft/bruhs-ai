use super::{
    host::{now, WhiteboardHost},
    ui::BUTTON,
};
use crate::state::*;
use crate::ui::dialog::Dialog;
use bruhs_models::whiteboard::*;
use leptos::prelude::*;
use std::collections::BTreeMap;
#[component]
pub(crate) fn BoardNotices() -> impl IntoView {
    let host = expect_context::<WhiteboardHost>();
    view! {<Show when=move||host.proposals.with(|p|p.iter().any(|p|host.statuses.with(|s|s.get(&p.id)==Some(&ProposalStatus::Pending))))><button class="bruhs-control fixed bottom-4 right-4 z-50 min-h-control rounded-grape border border-persimmon-400 bg-longan-900 px-4 text-label text-rambutan-100" on:click=move|_|{let next=host.proposals.with_untracked(|p|p.iter().find(|p|host.statuses.with_untracked(|s|s.get(&p.id)==Some(&ProposalStatus::Pending))).map(|p|p.board_id.clone()));host.route(next);}>"Review whiteboard proposal"</button></Show>}
}
#[component]
pub(crate) fn ProposalReviews(id: String) -> impl IntoView {
    let host = expect_context::<WhiteboardHost>();
    let id = StoredValue::new(id);
    let reviewing = RwSignal::new(None::<BoardProposal>);
    let error = RwSignal::new(None::<String>);
    view! {
        <div class="shrink-0">{move||host.proposals.get().into_iter().filter(|p|p.board_id==id.get_value()&&host.statuses.with(|s|s.get(&p.id)==Some(&ProposalStatus::Pending))).map(|p|{let description=p.description.clone();view!{<button class=BUTTON on:click=move|_|reviewing.set(Some(p.clone()))>{format!("Review proposal: {description}")}</button>}}).collect_view()}</div>
        {move||reviewing.get().map(|proposal|{
            host.changed.track();
            let before=host.seat(&proposal.board_id).map(|s|{let s=s.borrow();(s.input.editor.document().clone(),s.payloads.clone())});
            let result=before.ok_or(BoardError::Missing).and_then(|(board,payloads)|{let after=proposal.preview(&board,now())?;let changes=proposal.changes(&board)?;Ok((export_svg(&board,&payloads,None)?,export_svg(&after,&payloads,None)?,changes,serde_json::json!({"title":board.title,"background":board.background,"sources":board.sources,"assets":board.assets})))});
            let accept=StoredValue::new(proposal.clone());let reject=StoredValue::new(proposal.id.clone());let pending=host.statuses.with(|s|s.get(&proposal.id)==Some(&ProposalStatus::Pending));
            let (before_svg,after_svg,changes,valid)=match result{Ok((before,after,changes,metadata))=>(before,after,serde_json::to_string_pretty(&serde_json::json!({"before_metadata":metadata,"changed_shapes":changes,"exact_operations":proposal.operations})).unwrap_or_default(),true),Err(e)=>(String::new(),String::new(),e.to_string(),false)};
            view!{<Dialog open=Signal::derive(move||reviewing.get().is_some()) label="Review exact agent edit" on_close=Callback::new(move|_|reviewing.set(None)) max_width="max-w-5xl"><div class="flex max-h-[calc(100dvh-6rem)] w-full max-w-5xl flex-col gap-3 overflow-auto rounded-grape border border-longan-700 bg-longan-900 p-4"><h2 class="text-h4">{proposal.description.clone()}</h2><p class="text-body-sm text-rambutan-400">{format!("Revision {} · thread {} · immutable proposal {}",proposal.base_revision,proposal.requester.thread_id,proposal.id)}</p><div class="grid min-h-0 gap-3 md:grid-cols-2"><section><h3>"Before"</h3><div class="whiteboard-preview" inner_html=before_svg.clone()></div></section><section><h3>"After"</h3><div class="whiteboard-preview" inner_html=after_svg.clone()></div></section></div><details><summary>"Exact changed records and deletions"</summary><pre class="max-h-60 overflow-auto whitespace-pre-wrap break-all text-[12px]">{changes.clone()}</pre></details><div class="flex flex-wrap gap-2"><button class=BUTTON disabled=!valid||!pending on:click=move|_|{host.accept(accept.get_value());reviewing.set(None);}>"Accept exact edit"</button><button class=BUTTON disabled=!pending on:click=move|_|{host.statuses.update(|s|{s.insert(reject.get_value(),ProposalStatus::Rejected);});reviewing.set(None);}>"Reject"</button><button class=BUTTON on:click=move|_|reviewing.set(None)>"Close preview"</button></div>{move||error.get().map(|e|view!{<p role="alert">{e}</p>})}</div></Dialog>}
        })}
    }
}
#[component]
pub(crate) fn MermaidPreview(project: Memo<Option<String>>) -> impl IntoView {
    let host = expect_context::<WhiteboardHost>();
    let preview = RwSignal::new(None::<PortableBoard>);
    let error = RwSignal::new(None::<String>);
    let thread = expect_context::<ActiveThreadId>().0;
    Effect::new(move |_| {
        let Some(text) = host.mermaid.get() else {
            return;
        };
        let Some(project) = project.get() else {
            error.set(Some(
                "Choose a project folder to open this diagram as an editable board.".into(),
            ));
            return;
        };
        let result =
            import_mermaid(&text, project, "Chat diagram".into(), now()).map(|mut board| {
                if let Some(id) = thread.get_untracked() {
                    for shape in board.shapes.values_mut() {
                        shape.links.push(BoardLink::Thread { id: id.clone() });
                    }
                }
                PortableBoard {
                    board,
                    payloads: BTreeMap::new(),
                }
            });
        match result {
            Ok(board) => {
                preview.set(Some(board));
                error.set(None);
            }
            Err(e) => error.set(Some(e.to_string())),
        }
    });
    view! {<Dialog open=Signal::derive(move||host.mermaid.get().is_some()) label="Open chat diagram as board" on_close=Callback::new(move|_|host.mermaid.set(None)) max_width="max-w-3xl"><div class="flex max-h-[calc(100dvh-6rem)] w-full max-w-3xl flex-col gap-3 overflow-auto rounded-grape border border-longan-700 bg-longan-900 p-4"><h2 class="text-h4">"Open as editable board"</h2>{move||error.get().map(|e|view!{<p role="alert">{e}</p>})}{move||preview.get().map(|p|{let svg=export_svg(&p.board,&p.payloads,None).unwrap_or_default();view!{<p>{format!("{} editable shapes · private local copy",p.board.shapes.len())}</p><div class="whiteboard-preview" inner_html=svg></div>}})}<div class="flex gap-2"><button class=BUTTON disabled=move||preview.get().is_none() on:click=move|_|{if let Some(p)=preview.get_untracked(){let id=p.board.id.clone();match host.insert(p,None){Ok(_)=>{host.mermaid.set(None);host.save(&id,false);host.route(Some(id));},Err(e)=>error.set(Some(e.to_string()))}}}>"Save private board"</button><button class=BUTTON on:click=move|_|{preview.set(None);host.mermaid.set(None);}>"Cancel"</button></div></div></Dialog>}
}
