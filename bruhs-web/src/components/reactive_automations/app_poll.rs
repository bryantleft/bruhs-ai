//! The App source's fields (D20): what to watch, the account to poll with,
//! the repository and how often. A poll runs as the owner, so its accounts
//! are the owner's: Global GitHub accounts for the GitHub watches and the
//! connected mailboxes for new email. Every rule and line of copy is
//! `bruhs_models::apps`', so the desktop editor matches.
//!
//! Also the confirmation an editor asks before it changes the type of a
//! newer Bruhs' source, which replaces what that Bruhs saved.

use bruhs_models::apps::{
    poll_account_choices, poll_account_options, poll_form_with_watch, watch_label, AppPollForm,
    AppsListing, PollWatch, APPS_LOADING, CHANGE_NEWER_SOURCE, CHANGE_TYPE, MINUTES_LABEL,
    NO_GITHUB_ACCOUNTS, NO_MAILBOXES, POLL_ACCOUNT_LABEL, REPO_HINT, REPO_LABEL, WATCH_LABEL,
};
use bruhs_models::vault::KEEP;
use leptos::prelude::*;

use super::SELECT;
use crate::ui::{Button, ButtonSize, ButtonVariant, Input};

const LABEL: &str = "block min-w-0 space-y-1 text-label-sm text-rambutan-300";
const HINT: &str = "text-label-sm text-rambutan-400";
/// The fields' grid: a full row of the editor, two columns from `lg`.
const POLL_GRID: &str = "grid min-w-0 gap-3 sm:col-span-2 lg:grid-cols-2";
/// A line across both of the fields' columns.
const POLL_FULL_ROW: &str = "lg:col-span-2";
const WATCHES: [PollWatch; 3] = [
    PollWatch::GithubIssues,
    PollWatch::GithubPulls,
    PollWatch::NewMail,
];

/// The watch's value in the select.
fn watch_key(watch: PollWatch) -> &'static str {
    match watch {
        PollWatch::GithubIssues => "github_issues",
        PollWatch::GithubPulls => "github_pulls",
        PollWatch::NewMail => "new_mail",
    }
}

fn watch_for(key: &str) -> PollWatch {
    WATCHES
        .into_iter()
        .find(|watch| watch_key(*watch) == key)
        .unwrap_or_default()
}

#[component]
pub fn AppPollFields(
    form: RwSignal<AppPollForm>,
    listing: Signal<Option<AppsListing>>,
    /// Why the accounts can't be listed, such as a page on another address.
    #[prop(optional, into)]
    notice: Signal<Option<String>>,
    /// Why the fields are off, such as an older service.
    #[prop(optional, into)]
    disabled: Signal<Option<&'static str>>,
) -> impl IntoView {
    let off = Signal::derive(move || disabled.with(Option::is_some));
    let described = move || disabled.with(Option::is_some).then_some("app-poll-off");
    let watch = Signal::derive(move || form.with(|form| form.watch));
    let options = Memo::new(move |_| {
        listing.with(|listing| {
            listing
                .as_ref()
                .map(|listing| form.with(|form| poll_account_options(form, listing)))
        })
    });
    // Nothing to choose: say where accounts come from.
    let empty = move || {
        let watch = watch.get();
        listing
            .with(|listing| {
                listing
                    .as_ref()
                    .is_some_and(|listing| poll_account_choices(watch, listing).is_empty())
            })
            .then_some(if watch == PollWatch::NewMail {
                NO_MAILBOXES
            } else {
                NO_GITHUB_ACCOUNTS
            })
    };
    let status = move || {
        notice.get().or_else(|| {
            listing
                .with(Option::is_none)
                .then(|| APPS_LOADING.to_owned())
        })
    };
    view! {
        <div data-app-poll="" class=POLL_GRID>
            {move || disabled.get().map(|reason| view! { <p id="app-poll-off" role="status" data-app-poll-off="" class=format!("text-label-sm text-durian-300 {POLL_FULL_ROW}")>{reason}</p> })}
            <label class=LABEL>
                <span>{WATCH_LABEL}</span>
                <select
                    aria-label=WATCH_LABEL
                    data-app-poll-watch=""
                    class=SELECT
                    disabled=move || off.get()
                    title=move || disabled.get()
                    aria-describedby=described
                    prop:value=move || watch_key(watch.get())
                    on:change=move |event| {
                        let next = watch_for(&event_target_value(&event));
                        let current = form.get_untracked();
                        form.set(listing.with_untracked(|listing| poll_form_with_watch(&current, next, listing.as_ref())));
                    }
                >
                    {WATCHES.into_iter().map(|option| view! { <option value=watch_key(option) selected=move || watch.get() == option>{watch_label(option)}</option> }).collect_view()}
                </select>
            </label>
            <label class=LABEL>
                <span>{POLL_ACCOUNT_LABEL}</span>
                <select
                    aria-label=POLL_ACCOUNT_LABEL
                    data-app-poll-account=""
                    class=SELECT
                    disabled=move || off.get() || options.with(Option::is_none)
                    title=move || disabled.get().map(str::to_owned).or_else(status)
                    aria-describedby=move || if disabled.with(Option::is_some) { "app-poll-off" } else { "app-poll-status" }
                    prop:value=move || form.with(|form| form.account.clone())
                    on:change=move |event| {
                        let account = event_target_value(&event);
                        form.update(|form| form.account = account);
                    }
                >
                    {move || options.get().unwrap_or_default().into_iter().map(|(id, label)| { let chosen = id.clone(); view! { <option value=id selected=move || form.with(|form| form.account == chosen)>{label}</option> } }).collect_view()}
                </select>
                <p id="app-poll-status" role="status" class=format!("{HINT} empty:hidden")>{move || status().or_else(|| empty().map(str::to_owned))}</p>
            </label>
            <Show when=move || watch.get() != PollWatch::NewMail>
                <label class=LABEL>
                    <span>{REPO_LABEL}</span>
                    <Input
                        aria_label=REPO_LABEL
                        placeholder=REPO_HINT
                        value=Signal::derive(move || form.with(|form| form.repo.clone()))
                        on_input=Callback::new(move |text| form.update(|form| form.repo = text))
                        disabled=off
                        attr:data-app-poll-repo=""
                        attr:title=move || disabled.get()
                        attr:aria-describedby=described
                    />
                </label>
            </Show>
            <label class=LABEL>
                <span>{MINUTES_LABEL}</span>
                <Input
                    aria_label=MINUTES_LABEL
                    value=Signal::derive(move || form.with(|form| form.minutes.clone()))
                    on_input=Callback::new(move |text| form.update(|form| form.minutes = text))
                    disabled=off
                    attr:inputmode="numeric"
                    attr:data-app-poll-minutes=""
                    attr:title=move || disabled.get()
                    attr:aria-describedby=described
                />
            </label>
        </div>
    }
    .into_any()
}

/// Asks before a newer Bruhs' source changes type. `pending` holds the
/// type the owner picked; Change type runs `on_confirm` with it, Keep
/// drops it.
#[component]
pub fn NewerSourceConfirm(
    pending: RwSignal<Option<String>>,
    on_confirm: Callback<String>,
) -> impl IntoView {
    view! {
        <Show when=move || pending.with(Option::is_some)>
            <div role="group" aria-labelledby="newer-source-question" data-newer-source-confirm="" class="flex min-w-0 flex-wrap items-center gap-2 rounded-grape bg-durian-500/10 p-3 sm:col-span-2">
                <p id="newer-source-question" role="alert" class="min-w-0 text-body-sm text-durian-200">{CHANGE_NEWER_SOURCE}</p>
                <div class="flex flex-wrap gap-2">
                    <Button size=ButtonSize::Small on_click=Callback::new(move |_| { if let Some(kind) = pending.get_untracked() { pending.set(None); on_confirm.run(kind); } }) attr:data-newer-source-change="">{CHANGE_TYPE}</Button>
                    <Button variant=ButtonVariant::Ghost size=ButtonSize::Small on_click=Callback::new(move |_| pending.set(None)) attr:data-newer-source-keep="">{KEEP}</Button>
                </div>
            </div>
        </Show>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    // The account choices and watch changes are `bruhs_models::apps`',
    // tested there.
    use super::*;

    /// At 640×480 the bruh page's column is about 350px wide, and two
    /// columns from `sm` cut the selects to "New GitHub is" and "GitHub
    /// octo-0". The fields stack until `lg`, as on the desktop, and a full
    /// row never adds a column to a one-column grid.
    #[test]
    fn the_fields_stack_until_lg() {
        assert!(POLL_GRID.contains("lg:grid-cols-2"), "{POLL_GRID}");
        assert!(!POLL_GRID.contains("sm:grid-cols-2"), "{POLL_GRID}");
        assert!(
            POLL_GRID.contains("sm:col-span-2"),
            "a full row of the editor"
        );
        assert!(POLL_FULL_ROW.contains("lg:col-span-2"), "{POLL_FULL_ROW}");
        assert!(!POLL_FULL_ROW.contains("sm:col-span-2"), "{POLL_FULL_ROW}");
    }

    #[test]
    fn watch_keys_round_trip() {
        for watch in WATCHES {
            assert_eq!(watch_for(watch_key(watch)), watch);
        }
    }
}
