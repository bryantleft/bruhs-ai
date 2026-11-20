//! The App source's fields (D20): what to watch, the account to poll with,
//! the repository and how often. A poll runs as the owner, so its accounts
//! are the owner's: Global GitHub accounts for the GitHub watches and the
//! connected mailboxes for new email. Every rule and line of copy is
//! `bruhs_models::apps`', so the desktop editor matches.
//!
//! Also the confirmation an editor asks before it changes the type of a
//! newer Bruhs' source, which replaces what that Bruhs saved.

use bruhs_models::apps::{
    poll_account_choices, watch_label, AppPollForm, AppsListing, PollWatch, APPS_LOADING,
    APP_POLL_ACCOUNT_GONE, CHANGE_NEWER_SOURCE, CHANGE_TYPE, MINUTES_LABEL, NO_GITHUB_ACCOUNTS,
    NO_MAILBOXES, POLL_ACCOUNT_LABEL, POLL_ACCOUNT_NEEDED, REPO_HINT, REPO_LABEL, WATCH_LABEL,
};
use bruhs_models::vault::KEEP;
use leptos::prelude::*;

use super::SELECT;
use crate::ui::{Button, ButtonSize, ButtonVariant, Input};

const LABEL: &str = "block min-w-0 space-y-1 text-label-sm text-rambutan-300";
const HINT: &str = "text-label-sm text-rambutan-400";
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

/// The account select's options as `(id, label)`: "Choose an account."
/// first, then the watch's accounts. An account the form holds that the
/// listing no longer offers shows as gone, so the select never claims a
/// choice it doesn't have.
fn account_options(form: &AppPollForm, listing: &AppsListing) -> Vec<(String, String)> {
    let choices = poll_account_choices(form.watch, listing);
    let gone = (!form.account.is_empty() && !choices.iter().any(|(id, _)| *id == form.account))
        .then(|| (form.account.clone(), APP_POLL_ACCOUNT_GONE.to_owned()));
    std::iter::once((String::new(), POLL_ACCOUNT_NEEDED.to_owned()))
        .chain(gone)
        .chain(choices)
        .collect()
}

/// `form` with `watch` chosen. The account stays only while the new watch
/// still offers it, since a GitHub account can't watch mail.
fn with_watch(form: &AppPollForm, watch: PollWatch, listing: Option<&AppsListing>) -> AppPollForm {
    let keeps = listing.is_some_and(|listing| {
        poll_account_choices(watch, listing)
            .iter()
            .any(|(id, _)| *id == form.account)
    });
    AppPollForm {
        watch,
        account: if keeps {
            form.account.clone()
        } else {
            String::new()
        },
        ..form.clone()
    }
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
                .map(|listing| form.with(|form| account_options(form, listing)))
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
        <div data-app-poll="" class="grid min-w-0 gap-3 sm:col-span-2 sm:grid-cols-2">
            {move || disabled.get().map(|reason| view! { <p id="app-poll-off" role="status" data-app-poll-off="" class="text-label-sm text-durian-300 sm:col-span-2">{reason}</p> })}
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
                        form.set(listing.with_untracked(|listing| with_watch(&current, next, listing.as_ref())));
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
    use super::*;
    use bruhs_models::apps::{AppAccount, AppMailbox};
    use bruhs_models::connections::ServiceProvider;
    use bruhs_models::secrets::SavedCredentialState;
    use bruhs_models::vault::CredentialScope;

    fn listing() -> AppsListing {
        let account = |id: &str, app: &str, name: &str, scope: CredentialScope| AppAccount {
            id: id.into(),
            app: app.into(),
            name: name.into(),
            account: "octo".into(),
            scope,
            state: SavedCredentialState::Saved,
            attention: None,
        };
        AppsListing {
            accounts: vec![
                account("v1-g", "github", "GitHub octo", CredentialScope::Global),
                account(
                    "v1-o",
                    "github",
                    "Own",
                    CredentialScope::Bruh {
                        agent_id: "a".into(),
                    },
                ),
                account("v1-n", "notion", "Notes", CredentialScope::Global),
            ],
            mailboxes: vec![AppMailbox {
                mailbox_id: "m1".into(),
                provider: ServiceProvider::Gmail,
                account: "me@x.test".into(),
            }],
            ..AppsListing::default()
        }
    }

    #[test]
    fn the_app_source_offers_the_accounts_its_watch_needs() {
        let listing = listing();
        let mut form = AppPollForm::default();
        assert_eq!(
            account_options(&form, &listing),
            [
                (String::new(), POLL_ACCOUNT_NEEDED.to_owned()),
                ("v1-g".to_owned(), "GitHub octo".to_owned()),
            ]
        );
        form.watch = PollWatch::NewMail;
        assert_eq!(
            account_options(&form, &listing),
            [
                (String::new(), POLL_ACCOUNT_NEEDED.to_owned()),
                ("m1".to_owned(), "me@x.test, Gmail".to_owned()),
            ]
        );
        // A gone account shows as gone instead of as a choice it isn't.
        form.account = "m9".into();
        assert_eq!(
            account_options(&form, &listing)[1],
            ("m9".to_owned(), APP_POLL_ACCOUNT_GONE.to_owned())
        );
    }

    #[test]
    fn a_watch_change_keeps_only_an_account_it_offers() {
        let listing = listing();
        let form = AppPollForm {
            watch: PollWatch::GithubIssues,
            account: "v1-g".into(),
            repo: "o/r".into(),
            minutes: "5".into(),
        };
        let pulls = with_watch(&form, PollWatch::GithubPulls, Some(&listing));
        assert_eq!(
            (pulls.account.as_str(), pulls.repo.as_str()),
            ("v1-g", "o/r")
        );
        let mail = with_watch(&form, PollWatch::NewMail, Some(&listing));
        assert_eq!(
            (mail.watch, mail.account.as_str()),
            (PollWatch::NewMail, "")
        );
        assert_eq!(with_watch(&form, PollWatch::GithubPulls, None).account, "");
        for watch in WATCHES {
            assert_eq!(watch_for(watch_key(watch)), watch);
        }
    }
}
