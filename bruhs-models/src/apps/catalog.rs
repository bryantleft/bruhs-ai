//! The fourteen connectors (D3, D4): where each app lives, how its token is
//! sent, the values an account needs, the call that tests a token, and the
//! allowlist of endpoints a bruh may call, each with its class (ruling A).
//!
//! The allowlist is default deny: an endpoint no row names is refused, so
//! aliases (GitHub `/repositories/{id}`, Jira `/rest/api/2`), credential,
//! key and webhook endpoints, and money or delivery actions the catalog
//! doesn't name never reach an app. Each list starts with the common reads
//! plus the writes and sends its guide documents; growing one is a catalog
//! change with its test.
//!
//! Step 3 of the plan checked each row against the app's public docs on
//! 2026-11-19. Changes from the plan: Shopify's version is 2026-10 (F27),
//! Jira names the account by display name when its email is hidden, and
//! Figma's token help link points at its current docs.

use super::guides;
use super::{
    AppAction, EMAIL_SHAPE, ENTER_TOKEN, PARAM_EMAIL, PARAM_KEY, PARAM_SHOP, PARAM_SITE,
    PARAM_TOKEN, SHOP_HINT, SITE_HINT, SITE_SHAPE, TOKEN_SHAPE,
};
use std::collections::BTreeMap;

/// Every app call carries this `User-Agent` (GitHub requires one).
pub const USER_AGENT: &str = "bruhs-ai";

/// How the token reaches the app (D4). The spec's `Query(name)` is left out:
/// a token in a URL leaks into error text and logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppAuth {
    /// `Authorization: Bearer <token>`.
    Bearer,
    /// `Authorization: Basic base64(<email>:<token>)`.
    Basic,
    /// The raw token in this header.
    Header(&'static str),
    /// `Authorization: OAuth oauth_consumer_key="<key>", oauth_token="<token>"`.
    Trello,
}

/// Where an app lives: one host, or one DNS label the account names under a
/// fixed suffix. Always https on the default port.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppHost {
    Fixed(&'static str),
    /// `{label}{suffix}`, the label from the account's `param` value.
    Account {
        param: &'static str,
        suffix: &'static str,
    },
}

/// A value an account needs. Secret ones (`token`, `key`) go to the keychain;
/// the rest (`email`, `site`, `shop`) are the account's metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppParam {
    pub id: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub hint: &'static str,
}

/// The call that proves a token works and names its account.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppTest {
    pub method: &'static str,
    pub path: &'static str,
    pub body: Option<&'static str>,
    /// JSON pointers tried in order; the first holding a string or number
    /// names the account.
    pub account: &'static [&'static str],
    /// A pointer that must hold `true`, for apps that answer 200 on failure.
    pub ok: Option<&'static str>,
    /// What tells this account apart from the same person's account in
    /// another site, shop or workspace ([`super::account_for`]).
    pub tenant: AccountTenant,
    /// Whether every token of one account names the same account. False
    /// for an app whose test call names the token itself (Cloudflare's
    /// token id): a replacement can't be checked against the stored
    /// account, so it keeps the stored one.
    pub stable_account: bool,
}

/// The tenant part of an account's stored label (ruling A Task 8 I2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountTenant {
    /// The test call's label alone: one account per person.
    None,
    /// "{label} on {param}": a Jira site.
    Site(&'static str),
    /// The param alone: a Shopify shop, whose DNS label is unique and
    /// stays when the shop is renamed.
    Param(&'static str),
    /// "{label} in {value}", the value at this pointer of the test reply,
    /// when it names one: a Slack, Linear or Notion workspace.
    Workspace(&'static str),
}

/// One allowlisted endpoint: upper-case methods plus a path glob
/// ([`super::path_glob`]), and the class a matching call has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointRule {
    pub methods: &'static [&'static str],
    pub path: &'static str,
    pub action: AppAction,
    /// Query or top-level body keys (any case, `key[…]` included) a call to
    /// this endpoint may not send: [`super::APP_FIELD_REFUSED`].
    pub refused_keys: &'static [&'static str],
}

impl EndpointRule {
    /// The same endpoint, refusing these keys.
    const fn refusing(self, refused_keys: &'static [&'static str]) -> Self {
        EndpointRule {
            refused_keys,
            ..self
        }
    }
}

/// The app's GraphQL path. It takes only a POST whose body holds the
/// document (and its variables and operation name) and no URL query; a
/// query operation is a read and anything else is `mutation`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GraphqlRule {
    pub path: &'static str,
    /// The class of a mutation, or the model-facing copy that refuses every
    /// mutation (GitHub's, whose writes go by REST: final review A1 I1).
    pub mutation: Result<AppAction, &'static str>,
    /// Name globs ([`super::path_glob`]'s `*`, any case) that no word of the
    /// document may match, in a query or a mutation:
    /// [`super::APP_CREDENTIAL_REFUSED`]. GraphQL selects a field only by its
    /// literal name, so an alias or a fragment can't hide one (A1 I2).
    pub refused_names: &'static [&'static str],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AppConnector {
    pub id: &'static str,
    pub name: &'static str,
    pub host: AppHost,
    pub auth: AppAuth,
    /// Sent with every call, beside the auth header and [`USER_AGENT`].
    pub headers: &'static [(&'static str, &'static str)],
    pub params: &'static [AppParam],
    pub token_help_url: &'static str,
    /// App copy: the least the token needs.
    pub scopes_hint: &'static str,
    pub test: AppTest,
    /// The endpoints a bruh may call; first match wins, anything else is
    /// refused with `unlisted`.
    pub endpoints: &'static [EndpointRule],
    pub graphql: Option<GraphqlRule>,
    /// Model-facing: "Bruhs can't call that endpoint of {name} yet."
    pub unlisted: &'static str,
    /// Model-facing; at most 800 bytes.
    pub guide: &'static str,
}

/// The shared refusal for an endpoint an app's allowlist doesn't name.
macro_rules! unlisted {
    ($name:literal) => {
        concat!("Bruhs can't call that endpoint of ", $name, " yet.")
    };
}

const TOKEN: AppParam = AppParam {
    id: "token",
    label: PARAM_TOKEN,
    secret: true,
    hint: "",
};
const KEY: AppParam = AppParam {
    id: "key",
    label: PARAM_KEY,
    secret: true,
    hint: "",
};
const EMAIL: AppParam = AppParam {
    id: "email",
    label: PARAM_EMAIL,
    secret: false,
    hint: "",
};
const SITE: AppParam = AppParam {
    id: "site",
    label: PARAM_SITE,
    secret: false,
    hint: SITE_HINT,
};
const SHOP: AppParam = AppParam {
    id: "shop",
    label: PARAM_SHOP,
    secret: false,
    hint: SHOP_HINT,
};

const GET: &[&str] = &["GET"];
const GET_POST: &[&str] = &["GET", "POST"];
const POST: &[&str] = &["POST"];
const PUT: &[&str] = &["PUT"];
const PATCH: &[&str] = &["PATCH"];
const DELETE: &[&str] = &["DELETE"];
const POST_DELETE: &[&str] = &["POST", "DELETE"];
const PUT_PATCH_DELETE: &[&str] = &["PUT", "PATCH", "DELETE"];

const fn rule(
    methods: &'static [&'static str],
    path: &'static str,
    action: AppAction,
) -> EndpointRule {
    EndpointRule {
        methods,
        path,
        action,
        refused_keys: &[],
    }
}

const fn read(methods: &'static [&'static str], path: &'static str) -> EndpointRule {
    rule(methods, path, AppAction::Read)
}

const fn get(path: &'static str) -> EndpointRule {
    read(GET, path)
}

const fn write(methods: &'static [&'static str], path: &'static str) -> EndpointRule {
    rule(methods, path, AppAction::Write)
}

const fn send(methods: &'static [&'static str], path: &'static str) -> EndpointRule {
    rule(methods, path, AppAction::SendAsOwner)
}

const fn deliver(methods: &'static [&'static str], path: &'static str) -> EndpointRule {
    rule(methods, path, AppAction::Delivery)
}

const fn test_get(path: &'static str, account: &'static [&'static str]) -> AppTest {
    AppTest {
        method: "GET",
        path,
        body: None,
        account,
        ok: None,
        tenant: AccountTenant::None,
        stable_account: true,
    }
}

/// Todoist's assignee fields, new and old.
const TODOIST_ASSIGN: &[&str] = &["assignee_id", "responsible_uid"];
/// Slack fields that make a message show as another name or face.
const SLACK_PERSONA: &[&str] = &["username", "icon_url", "icon_emoji", "as_user"];

/// HubSpot's endpoints, with each named CRM object type's list and record
/// reads, search and batch reads, create and update. Types are named, so
/// invoices, quotes, payments and custom objects stay refused (final review
/// A1 M5).
macro_rules! hubspot_endpoints {
    ($($kind:literal),+) => {
        &[
            get("/account-info/v3/details"),
            get("/crm/v3/owners"),
            get("/crm/v3/properties/*"),
            $(
                get(concat!("/crm/v3/objects/", $kind)),
                get(concat!("/crm/v3/objects/", $kind, "/*")),
                read(POST, concat!("/crm/v3/objects/", $kind, "/search")),
                read(POST, concat!("/crm/v3/objects/", $kind, "/batch/read")),
                write(POST, concat!("/crm/v3/objects/", $kind)),
                write(PATCH, concat!("/crm/v3/objects/", $kind, "/*")),
            )+
            send(POST, "/marketing/v3/transactional/single-email/send"),
            send(POST, "/conversations/v3/conversations/threads/*/messages"),
        ]
    };
}

/// The Shopify Admin API version every Shopify call and guide uses: the
/// latest stable on 2026-11-19, served until October 2027 (F27).
pub const SHOPIFY_API_VERSION: &str = "2026-10";

pub const CONNECTORS: [AppConnector; 14] = [
    AppConnector {
        id: "github",
        name: "GitHub",
        host: AppHost::Fixed("api.github.com"),
        auth: AppAuth::Bearer,
        headers: &[
            ("Accept", "application/vnd.github+json"),
            ("X-GitHub-Api-Version", "2022-11-28"),
        ],
        params: &[TOKEN],
        token_help_url: "https://github.com/settings/personal-access-tokens",
        scopes_hint: "A fine-grained token for the repositories a bruh works on: issues and pull requests, read or write as you choose. It's separate from the GitHub sign-in in Settings, Vault.",
        test: test_get("/user", &["/login"]),
        endpoints: &[
            get("/user"),
            get("/user/repos"),
            get("/orgs/*/repos"),
            get("/notifications"),
            get("/search/issues"),
            get("/search/code"),
            get("/search/repositories"),
            get("/repos/*/*"),
            get("/repos/*/*/issues"),
            get("/repos/*/*/issues/*"),
            get("/repos/*/*/issues/*/comments"),
            get("/repos/*/*/pulls"),
            get("/repos/*/*/pulls/*"),
            get("/repos/*/*/pulls/*/files"),
            get("/repos/*/*/pulls/*/commits"),
            get("/repos/*/*/pulls/*/reviews"),
            get("/repos/*/*/pulls/*/comments"),
            get("/repos/*/*/commits"),
            get("/repos/*/*/commits/*"),
            get("/repos/*/*/branches"),
            get("/repos/*/*/branches/*"),
            get("/repos/*/*/contents/**"),
            get("/repos/*/*/labels"),
            get("/repos/*/*/milestones"),
            get("/repos/*/*/releases"),
            get("/repos/*/*/releases/*"),
            get("/repos/*/*/actions/runs"),
            get("/repos/*/*/actions/runs/*"),
            send(POST, "/repos/*/*/issues"),
            send(PATCH, "/repos/*/*/issues/*"),
            send(POST, "/repos/*/*/issues/*/comments"),
            send(POST, "/repos/*/*/issues/*/labels"),
            send(POST, "/repos/*/*/pulls/*/reviews"),
            deliver(POST, "/repos/*/*/pulls"),
            deliver(PUT, "/repos/*/*/pulls/*/merge"),
            deliver(PUT, "/repos/*/*/pulls/*/update-branch"),
            deliver(PUT, "/repos/*/*/contents/**"),
            deliver(POST, "/repos/*/*/git/refs"),
            deliver(PATCH, "/repos/*/*/git/refs/**"),
        ],
        graphql: Some(GraphqlRule {
            path: "/graphql",
            mutation: Err(super::APP_GITHUB_GRAPHQL_WRITES),
            refused_names: &[],
        }),
        unlisted: unlisted!("GitHub"),
        guide: guides::GITHUB,
    },
    AppConnector {
        id: "notion",
        name: "Notion",
        host: AppHost::Fixed("api.notion.com"),
        auth: AppAuth::Bearer,
        headers: &[("Notion-Version", "2022-06-28")],
        params: &[TOKEN],
        token_help_url: "https://www.notion.so/profile/integrations",
        scopes_hint: "An internal integration's secret. Share only the pages and databases a bruh needs with the integration, and leave out comment access unless it should comment.",
        // An internal integration's bot user names its workspace.
        test: AppTest {
            tenant: AccountTenant::Workspace("/bot/workspace_name"),
            ..test_get("/v1/users/me", &["/name"])
        },
        endpoints: &[
            get("/v1/users"),
            get("/v1/users/*"),
            get("/v1/pages/*"),
            get("/v1/pages/*/properties/*"),
            get("/v1/blocks/*"),
            get("/v1/blocks/*/children"),
            get("/v1/databases/*"),
            get("/v1/data_sources/*"),
            get("/v1/comments"),
            read(POST, "/v1/search"),
            read(POST, "/v1/databases/*/query"),
            read(POST, "/v1/data_sources/*/query"),
            write(POST, "/v1/pages"),
            write(PATCH, "/v1/pages/*"),
            write(PATCH, "/v1/blocks/*"),
            write(PATCH, "/v1/blocks/*/children"),
            write(DELETE, "/v1/blocks/*"),
            send(POST, "/v1/comments"),
        ],
        graphql: None,
        unlisted: unlisted!("Notion"),
        guide: guides::NOTION,
    },
    AppConnector {
        id: "linear",
        name: "Linear",
        host: AppHost::Fixed("api.linear.app"),
        auth: AppAuth::Header("Authorization"),
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://linear.app/settings/account/security",
        scopes_hint: "A personal API key with read access, plus write only if a bruh files or changes issues, limited to the teams it works with.",
        test: AppTest {
            method: "POST",
            path: "/graphql",
            body: Some(r#"{"query":"{ viewer { email organization { urlKey } } }"}"#),
            account: &["/data/viewer/email"],
            ok: None,
            // An API key belongs to one workspace; its URL key is unique
            // and readable.
            tenant: AccountTenant::Workspace("/data/viewer/organization/urlKey"),
            stable_account: true,
        },
        endpoints: &[],
        graphql: Some(GraphqlRule {
            path: "/graphql",
            mutation: Ok(AppAction::SendAsOwner),
            // Webhooks (and their signing secrets), API keys and OAuth clients.
            refused_names: &["webhook*", "*secret*", "apiKey*", "oauthClient*"],
        }),
        unlisted: unlisted!("Linear"),
        guide: guides::LINEAR,
    },
    AppConnector {
        id: "airtable",
        name: "Airtable",
        host: AppHost::Fixed("api.airtable.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://airtable.com/create/tokens",
        scopes_hint: "A personal access token with data.records:read and schema.bases:read on only the bases a bruh needs. Add data.records:write if it changes records, and user.email:read to show your email here.",
        test: test_get("/v0/meta/whoami", &["/email", "/id"]),
        // Base ids start with `app`, which keeps `/v0/bases/…` (webhooks) out.
        endpoints: &[
            get("/v0/meta/whoami"),
            get("/v0/meta/bases"),
            get("/v0/meta/bases/app*/tables"),
            get("/v0/app*/*"),
            get("/v0/app*/*/*"),
            read(POST, "/v0/app*/*/listRecords"),
            write(POST, "/v0/app*/*"),
            write(PATCH, "/v0/app*/*"),
            write(PATCH, "/v0/app*/*/*"),
            write(DELETE, "/v0/app*/*/*"),
        ],
        graphql: None,
        unlisted: unlisted!("Airtable"),
        guide: guides::AIRTABLE,
    },
    AppConnector {
        id: "todoist",
        name: "Todoist",
        host: AppHost::Fixed("api.todoist.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://app.todoist.com/app/settings/integrations/developer",
        scopes_hint: "Your API token from Settings, Integrations, Developer. Todoist tokens can't be narrowed, so a bruh can change anything you can.",
        test: test_get("/api/v1/user", &["/email"]),
        endpoints: &[
            get("/api/v1/user"),
            get("/api/v1/tasks"),
            get("/api/v1/tasks/*"),
            get("/api/v1/projects"),
            get("/api/v1/projects/*"),
            get("/api/v1/sections"),
            get("/api/v1/labels"),
            get("/api/v1/comments"),
            // Assigning a task notifies the assignee (final review A1 M4).
            write(POST, "/api/v1/tasks").refusing(TODOIST_ASSIGN),
            // Quick add's `note` posts a comment as the owner.
            send(POST, "/api/v1/tasks/quick"),
            write(POST, "/api/v1/tasks/*").refusing(TODOIST_ASSIGN),
            write(POST, "/api/v1/tasks/*/close"),
            write(POST, "/api/v1/tasks/*/reopen"),
            write(DELETE, "/api/v1/tasks/*"),
            send(POST, "/api/v1/comments"),
        ],
        graphql: None,
        unlisted: unlisted!("Todoist"),
        guide: guides::TODOIST,
    },
    AppConnector {
        id: "trello",
        name: "Trello",
        host: AppHost::Fixed("api.trello.com"),
        auth: AppAuth::Trello,
        headers: &[],
        params: &[TOKEN, KEY],
        token_help_url: "https://trello.com/power-ups/admin",
        scopes_hint: "A Power-Up's API key and a token you generate for it. Ask for read only if a bruh only reads boards.",
        test: test_get("/1/members/me", &["/username"]),
        endpoints: &[
            get("/1/members/me"),
            get("/1/members/me/boards"),
            get("/1/boards/*"),
            get("/1/boards/*/lists"),
            get("/1/boards/*/cards"),
            get("/1/lists/*/cards"),
            get("/1/cards/*"),
            get("/1/cards/*/actions"),
            // Adding members notifies them (final review A1 M4).
            write(POST, "/1/cards").refusing(&["idmembers"]),
            write(PUT, "/1/cards/*").refusing(&["idmembers"]),
            send(POST, "/1/cards/*/actions/comments"),
        ],
        graphql: None,
        unlisted: unlisted!("Trello"),
        guide: guides::TRELLO,
    },
    AppConnector {
        id: "clickup",
        name: "ClickUp",
        host: AppHost::Fixed("api.clickup.com"),
        auth: AppAuth::Header("Authorization"),
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://app.clickup.com/settings/apps",
        scopes_hint: "Your personal API token from Settings, Apps. ClickUp tokens can't be narrowed, so a bruh can change anything you can.",
        test: test_get("/api/v2/user", &["/user/email"]),
        endpoints: &[
            get("/api/v2/user"),
            get("/api/v2/team"),
            get("/api/v2/team/*/space"),
            get("/api/v2/space/*/folder"),
            get("/api/v2/space/*/list"),
            get("/api/v2/folder/*/list"),
            get("/api/v2/list/*"),
            get("/api/v2/list/*/task"),
            get("/api/v2/task/*"),
            get("/api/v2/task/*/comment"),
            // `notify_all` and assignees notify people (final review A1 M4).
            write(POST, "/api/v2/list/*/task").refusing(&["notify_all", "assignees"]),
            write(PUT, "/api/v2/task/*").refusing(&["notify_all", "assignees"]),
            send(POST, "/api/v2/task/*/comment"),
            send(POST, "/api/v2/list/*/comment"),
            send(POST, "/api/v2/view/*/comment"),
            send(POST, "/api/v3/workspaces/*/chat/channels/*/messages"),
        ],
        graphql: None,
        unlisted: unlisted!("ClickUp"),
        guide: guides::CLICKUP,
    },
    AppConnector {
        id: "hubspot",
        name: "HubSpot",
        host: AppHost::Fixed("api.hubapi.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://app.hubspot.com/private-apps",
        scopes_hint: "A private app's access token with only the scopes a bruh needs, such as crm.objects.contacts.read.",
        test: test_get("/account-info/v3/details", &["/portalId"]),
        endpoints: hubspot_endpoints!("contacts", "companies", "deals", "tickets", "notes", "tasks"),
        graphql: None,
        unlisted: unlisted!("HubSpot"),
        guide: guides::HUBSPOT,
    },
    AppConnector {
        id: "jira",
        name: "Jira",
        host: AppHost::Account {
            param: "site",
            suffix: ".atlassian.net",
        },
        auth: AppAuth::Basic,
        headers: &[("Accept", "application/json")],
        params: &[TOKEN, EMAIL, SITE],
        token_help_url: "https://id.atlassian.com/manage-profile/security/api-tokens",
        scopes_hint: "An API token for your Atlassian account, with scopes for the Jira work a bruh does if you can choose them. Use the email you sign in with.",
        // One Atlassian account signs in to every site.
        test: AppTest {
            tenant: AccountTenant::Site("site"),
            ..test_get("/rest/api/3/myself", &["/emailAddress", "/displayName"])
        },
        // Only `/rest/api/3`: `/rest/api/2` and `/rest/api/latest` are refused.
        endpoints: &[
            get("/rest/api/3/myself"),
            get("/rest/api/3/search/jql"),
            get("/rest/api/3/issue/*"),
            get("/rest/api/3/issue/*/comment"),
            get("/rest/api/3/issue/*/transitions"),
            get("/rest/api/3/project/search"),
            get("/rest/api/3/project/*"),
            read(POST, "/rest/api/3/search/jql"),
            read(POST, "/rest/api/3/issue/bulkfetch"),
            // An edit or transition can carry `update.comment`, which posts
            // and emails a comment as the owner.
            send(PUT, "/rest/api/3/issue/*"),
            send(POST, "/rest/api/3/issue/*/transitions"),
            send(POST, "/rest/api/3/issue"),
            send(POST, "/rest/api/3/issue/*/comment"),
            send(POST, "/rest/api/3/issue/*/notify"),
        ],
        graphql: None,
        unlisted: unlisted!("Jira"),
        guide: guides::JIRA,
    },
    AppConnector {
        id: "shopify",
        name: "Shopify",
        host: AppHost::Account {
            param: "shop",
            suffix: ".myshopify.com",
        },
        auth: AppAuth::Header("X-Shopify-Access-Token"),
        headers: &[],
        params: &[TOKEN, SHOP],
        token_help_url: "https://help.shopify.com/en/manual/apps/app-types/custom-apps",
        scopes_hint: "A custom app's Admin API access token with only the scopes a bruh needs, such as read_orders.",
        test: AppTest {
            tenant: AccountTenant::Param("shop"),
            ..test_get("/admin/api/2026-10/shop.json", &["/shop/name"])
        },
        // REST only under a version; GraphQL at any `/admin/…/graphql.json`.
        endpoints: &[
            get("/admin/api/*/shop.json"),
            get("/admin/api/*/orders.json"),
            get("/admin/api/*/orders/*.json"),
            get("/admin/api/*/products.json"),
            get("/admin/api/*/products/*.json"),
            get("/admin/api/*/customers.json"),
            get("/admin/api/*/customers/*.json"),
            write(POST, "/admin/api/*/products.json"),
            write(PUT, "/admin/api/*/products/*.json"),
            send(POST, "/admin/api/*/orders.json"),
            send(POST, "/admin/api/*/orders/*/cancel.json"),
            send(POST, "/admin/api/*/orders/*/refunds.json"),
            send(POST, "/admin/api/*/orders/*/transactions.json"),
            send(POST, "/admin/api/*/fulfillments.json"),
            send(POST, "/admin/api/*/draft_orders/*/send_invoice.json"),
            send(POST, "/admin/api/*/customers/*/send_invite.json"),
        ],
        graphql: Some(GraphqlRule {
            path: "/admin/**/graphql.json",
            mutation: Ok(AppAction::SendAsOwner),
            // Delegate and Storefront tokens (`accessToken` is their value)
            // and every kind of webhook subscription.
            refused_names: &["*accessToken*", "*webhookSubscription*"],
        }),
        unlisted: unlisted!("Shopify"),
        guide: guides::SHOPIFY,
    },
    AppConnector {
        id: "stripe",
        name: "Stripe",
        host: AppHost::Fixed("api.stripe.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://dashboard.stripe.com/apikeys",
        scopes_hint: "A restricted key with Read on your account and only the resources a bruh needs. Leave the rest at None.",
        test: test_get("/v1/account", &["/email", "/id"]),
        endpoints: &[
            get("/v1/account"),
            get("/v1/balance"),
            get("/v1/balance_transactions"),
            get("/v1/customers"),
            get("/v1/customers/*"),
            get("/v1/charges"),
            get("/v1/charges/*"),
            get("/v1/payment_intents"),
            get("/v1/payment_intents/*"),
            get("/v1/invoices"),
            get("/v1/invoices/*"),
            get("/v1/subscriptions"),
            get("/v1/subscriptions/*"),
            get("/v1/products"),
            get("/v1/products/*"),
            get("/v1/prices"),
            get("/v1/prices/*"),
            get("/v1/refunds"),
            get("/v1/refunds/*"),
            get("/v1/payouts"),
            get("/v1/disputes"),
            // A customer balance is charged on the next invoice.
            write(POST, "/v1/customers").refusing(&["balance"]),
            // A new email gets the customer's invoices, receipts and their
            // hosted links (final review A1 M4).
            write(POST, "/v1/customers/*").refusing(&["balance", "email"]),
            write(POST, "/v1/products"),
            write(POST, "/v1/products/*"),
            write(POST, "/v1/prices"),
            // Invoice items are swept into the next charge, and a draft with
            // auto_advance finalizes, then charges or emails the customer.
            send(POST, "/v1/invoiceitems"),
            send(POST, "/v1/invoices"),
            send(POST, "/v1/invoices/*"),
            send(POST, "/v1/charges"),
            send(POST, "/v1/charges/*/capture"),
            send(POST, "/v1/payment_intents"),
            send(POST, "/v1/payment_intents/*/confirm"),
            send(POST, "/v1/payment_intents/*/capture"),
            send(POST, "/v1/refunds"),
            send(POST, "/v1/payouts"),
            send(POST, "/v1/transfers"),
            send(POST, "/v1/transfers/*/reversals"),
            send(POST, "/v1/subscriptions"),
            send(POST_DELETE, "/v1/subscriptions/*"),
            send(POST, "/v1/invoices/*/send"),
            send(POST, "/v1/invoices/*/pay"),
            send(POST, "/v1/invoices/*/finalize"),
            send(POST, "/v1/invoices/*/void"),
            send(POST, "/v1/credit_notes"),
        ],
        graphql: None,
        unlisted: unlisted!("Stripe"),
        guide: guides::STRIPE,
    },
    AppConnector {
        id: "cloudflare",
        name: "Cloudflare",
        host: AppHost::Fixed("api.cloudflare.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://dash.cloudflare.com/profile/api-tokens",
        scopes_hint: "A user API token with only the permissions and zones a bruh needs, such as Zone DNS Read.",
        // Verify names the token, not the user: each token is its own id.
        test: AppTest {
            stable_account: false,
            ..test_get("/client/v4/user/tokens/verify", &["/result/id"])
        },
        endpoints: &[
            get("/client/v4/user/tokens/verify"),
            get("/client/v4/accounts"),
            get("/client/v4/accounts/*"),
            get("/client/v4/zones"),
            get("/client/v4/zones/*"),
            get("/client/v4/zones/*/dns_records"),
            get("/client/v4/zones/*/dns_records/*"),
            // Repointing MX or A records can capture the owner's mail and
            // password resets, so a DNS change asks like a send (A1 M4).
            send(POST, "/client/v4/zones/*/dns_records"),
            send(PUT_PATCH_DELETE, "/client/v4/zones/*/dns_records/*"),
        ],
        graphql: None,
        unlisted: unlisted!("Cloudflare"),
        guide: guides::CLOUDFLARE,
    },
    AppConnector {
        id: "slack",
        name: "Slack",
        host: AppHost::Fixed("slack.com"),
        auth: AppAuth::Bearer,
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://api.slack.com/apps",
        scopes_hint: "A user token to act as you or a bot token to post as the app, with only the scopes a bruh needs, such as channels:history.",
        test: AppTest {
            method: "GET",
            path: "/api/auth.test",
            body: None,
            account: &["/user"],
            ok: Some("/ok"),
            tenant: AccountTenant::Workspace("/team"),
            stable_account: true,
        },
        // Slack's Web API takes GET or POST for every method, posting
        // included, so each method is named with both.
        endpoints: &[
            read(GET_POST, "/api/auth.test"),
            read(GET_POST, "/api/conversations.list"),
            read(GET_POST, "/api/conversations.history"),
            read(GET_POST, "/api/conversations.replies"),
            read(GET_POST, "/api/conversations.info"),
            read(GET_POST, "/api/conversations.members"),
            read(GET_POST, "/api/users.list"),
            read(GET_POST, "/api/users.info"),
            read(GET_POST, "/api/users.lookupByEmail"),
            read(GET_POST, "/api/users.getPresence"),
            read(GET_POST, "/api/search.messages"),
            read(GET_POST, "/api/search.files"),
            read(GET_POST, "/api/team.info"),
            read(GET_POST, "/api/reactions.get"),
            read(GET_POST, "/api/pins.list"),
            // A bot token with chat:write.customize could post as anyone
            // (final review A1 M3).
            send(GET_POST, "/api/chat.postMessage").refusing(SLACK_PERSONA),
            send(GET_POST, "/api/chat.update").refusing(SLACK_PERSONA),
            send(GET_POST, "/api/chat.scheduleMessage").refusing(SLACK_PERSONA),
            send(GET_POST, "/api/reactions.add"),
            send(GET_POST, "/api/reactions.remove"),
        ],
        graphql: None,
        unlisted: unlisted!("Slack"),
        guide: guides::SLACK,
    },
    AppConnector {
        id: "figma",
        name: "Figma",
        host: AppHost::Fixed("api.figma.com"),
        auth: AppAuth::Header("X-Figma-Token"),
        headers: &[],
        params: &[TOKEN],
        token_help_url: "https://developers.figma.com/docs/rest-api/personal-access-tokens/",
        scopes_hint: "A personal access token with current_user:read and only the file scopes a bruh needs, read only if it only reads.",
        test: test_get("/v1/me", &["/email"]),
        endpoints: &[
            get("/v1/me"),
            get("/v1/files/*"),
            get("/v1/files/*/nodes"),
            get("/v1/files/*/versions"),
            get("/v1/files/*/comments"),
            get("/v1/images/*"),
            get("/v1/teams/*/projects"),
            get("/v1/projects/*/files"),
            send(POST, "/v1/files/*/comments"),
            send(POST, "/v1/files/*/comments/*/reactions"),
        ],
        graphql: None,
        unlisted: unlisted!("Figma"),
        guide: guides::FIGMA,
    },
];

/// The connector with this id.
pub fn connector(id: &str) -> Option<&'static AppConnector> {
    let all: &'static [AppConnector] = &CONNECTORS;
    all.iter().find(|connector| connector.id == id)
}

/// The host an account's calls go to: the fixed host, or the account's
/// label under the connector's suffix.
pub fn app_host(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
) -> Result<String, &'static str> {
    match connector.host {
        AppHost::Fixed(host) => Ok(host.to_string()),
        AppHost::Account { param, suffix } => {
            let spec = connector
                .params
                .iter()
                .find(|candidate| candidate.id == param)
                .ok_or(SITE_SHAPE)?;
            let value = params.get(param).ok_or(SITE_SHAPE)?;
            let label = validate_param(spec, value)?;
            Ok(format!("{label}{suffix}"))
        }
    }
}

/// `https://{host}`.
pub fn base_url(
    connector: &AppConnector,
    params: &BTreeMap<String, String>,
) -> Result<String, &'static str> {
    app_host(connector, params).map(|host| format!("https://{host}"))
}

/// Whether some connector may call this host: a fixed host, or exactly one
/// valid label under an account suffix. The egress resolver answers only
/// for these (D13).
pub fn host_declared(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    CONNECTORS.iter().any(|connector| match connector.host {
        AppHost::Fixed(fixed) => host == fixed,
        AppHost::Account { suffix, .. } => host
            .strip_suffix(suffix)
            .is_some_and(|label| dns_label(label).is_some()),
    })
}

/// Checks and normalizes one account value (D4): a token or key is trimmed,
/// then 8 bytes to 4 KiB of visible ASCII with no `"` or `\` (Trello quotes
/// both values in its header, and a header takes nothing else); an email
/// is trimmed; a site or shop is one DNS label, lower-cased.
pub fn validate_param(param: &AppParam, value: &str) -> Result<String, &'static str> {
    let value = value.trim();
    match param.id {
        "email" => valid_email(value)
            .then(|| value.to_string())
            .ok_or(EMAIL_SHAPE),
        "site" | "shop" => dns_label(value).ok_or(SITE_SHAPE),
        _ if value.is_empty() => Err(ENTER_TOKEN),
        _ => {
            let plain = value
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && !matches!(byte, b'"' | b'\\'));
            if (MIN_TOKEN_BYTES..=MAX_TOKEN_BYTES).contains(&value.len()) && plain {
                Ok(value.to_string())
            } else {
                Err(TOKEN_SHAPE)
            }
        }
    }
}

/// Shorter secrets can't be scrubbed from replies safely (F8).
const MIN_TOKEN_BYTES: usize = 8;
const MAX_TOKEN_BYTES: usize = 4 * 1024;

/// 3 to 254 characters with one `@` between two non-empty parts, and no
/// whitespace, control character or `:` (Basic auth ends the user at the
/// first colon).
fn valid_email(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    (3..=254).contains(&value.chars().count())
        && !local.is_empty()
        && !domain.is_empty()
        && !domain.contains('@')
        && !value
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == ':')
}

/// One DNS label, lower-cased: `[a-z0-9]` with inner hyphens, 1 to 63 long.
fn dns_label(value: &str) -> Option<String> {
    let label = value.to_ascii_lowercase();
    let shaped = (1..=63).contains(&label.len())
        && label
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        && !label.starts_with('-')
        && !label.ends_with('-');
    shaped.then_some(label)
}

#[cfg(test)]
mod tests {
    use super::super::{classify, validate_path, AppCall, APP_METHODS};
    use super::*;

    #[test]
    fn the_catalog_lists_the_fourteen_apps_with_fixed_hosts() {
        let ids: Vec<_> = CONNECTORS.iter().map(|c| c.id).collect();
        assert_eq!(
            ids,
            [
                "github",
                "notion",
                "linear",
                "airtable",
                "todoist",
                "trello",
                "clickup",
                "hubspot",
                "jira",
                "shopify",
                "stripe",
                "cloudflare",
                "slack",
                "figma"
            ]
        );
        assert_eq!(
            app_host(connector("github").unwrap(), &BTreeMap::new()),
            Ok("api.github.com".into())
        );
        let jira = connector("jira").unwrap();
        let site = |s: &str| {
            BTreeMap::from([
                ("site".to_string(), s.to_string()),
                ("email".into(), "me@acme.test".into()),
            ])
        };
        assert_eq!(
            app_host(jira, &site("Acme")),
            Ok("acme.atlassian.net".into())
        );
        for bad in [
            "acme.evil.com",
            "acme/x",
            "-acme",
            "",
            "a@b",
            "a..b",
            &"a".repeat(64),
        ] {
            assert!(app_host(jira, &site(bad)).is_err(), "{bad}");
        }
        assert!(
            host_declared("api.github.com")
                && host_declared("acme.atlassian.net")
                && host_declared("x.myshopify.com")
        );
        for host in [
            "evil.com",
            "a.b.atlassian.net",
            "atlassian.net",
            "api.github.com.evil.com",
            "localhost",
        ] {
            assert!(!host_declared(host), "{host}");
        }
        assert_eq!(connector("trello").unwrap().auth, AppAuth::Trello);
        assert!(CONNECTORS.iter().all(|c| c.guide.len() <= 800
            && c.token_help_url.starts_with("https://")
            && !c.scopes_hint.contains(" - ")
            && c.params.iter().any(|p| p.id == "token")));
        // Ruling A replaced `get_is_read` with allowlists: a Slack GET may
        // post, a GitHub GET of a listed endpoint is a read.
        let get = |app: &str, path: &str| {
            let call = AppCall {
                app: app.into(),
                account: None,
                method: "GET".into(),
                path: path.into(),
                query: BTreeMap::new(),
                body: None,
            };
            classify(connector(app).unwrap(), &call)
        };
        assert_eq!(
            get("slack", "/api/chat.postMessage"),
            Ok(AppAction::SendAsOwner)
        );
        assert_eq!(get("github", "/user"), Ok(AppAction::Read));
    }

    #[test]
    fn each_row_matches_its_app_docs() {
        // Checked against each app's public API docs on 2026-11-19 (D3).
        let row = |id: &str| connector(id).unwrap();
        let hosts: Vec<_> = CONNECTORS.iter().map(|c| (c.id, c.host)).collect();
        assert_eq!(
            hosts[..3],
            [
                ("github", AppHost::Fixed("api.github.com")),
                ("notion", AppHost::Fixed("api.notion.com")),
                ("linear", AppHost::Fixed("api.linear.app")),
            ]
        );
        assert_eq!(row("slack").host, AppHost::Fixed("slack.com"));
        assert_eq!(row("hubspot").host, AppHost::Fixed("api.hubapi.com"));
        // Linear and ClickUp personal keys go bare in Authorization.
        assert_eq!(row("linear").auth, AppAuth::Header("Authorization"));
        assert_eq!(row("clickup").auth, AppAuth::Header("Authorization"));
        assert_eq!(row("figma").auth, AppAuth::Header("X-Figma-Token"));
        assert_eq!(
            row("shopify").auth,
            AppAuth::Header("X-Shopify-Access-Token")
        );
        assert_eq!(row("jira").auth, AppAuth::Basic);
        assert_eq!(
            row("github").headers[1],
            ("X-GitHub-Api-Version", "2022-11-28")
        );
        assert_eq!(row("notion").headers, [("Notion-Version", "2022-06-28")]);
        // Shopify serves each version for a year: 2025-10 has lapsed (F27).
        assert_eq!(
            row("shopify").test.path,
            format!("/admin/api/{SHOPIFY_API_VERSION}/shop.json")
        );
        // Jira hides a user's email under some privacy settings.
        assert_eq!(row("jira").test.account, ["/emailAddress", "/displayName"]);
        // Airtable returns email only with user.email:read.
        assert_eq!(row("airtable").test.account, ["/email", "/id"]);
        assert_eq!(row("slack").test.ok, Some("/ok"));
        // The tenant each label names (ruling A Task 8 I2, M7). The
        // workspace pointers follow each app's docs; Task 19's live checks
        // confirm them.
        let tenants: Vec<_> = CONNECTORS
            .iter()
            .filter(|c| c.test.tenant != AccountTenant::None)
            .map(|c| (c.id, c.test.tenant))
            .collect();
        assert_eq!(
            tenants,
            [
                ("notion", AccountTenant::Workspace("/bot/workspace_name")),
                (
                    "linear",
                    AccountTenant::Workspace("/data/viewer/organization/urlKey")
                ),
                ("jira", AccountTenant::Site("site")),
                ("shopify", AccountTenant::Param("shop")),
                ("slack", AccountTenant::Workspace("/team")),
            ]
        );
        let unstable: Vec<_> = CONNECTORS
            .iter()
            .filter(|c| !c.test.stable_account)
            .map(|c| c.id)
            .collect();
        assert_eq!(unstable, ["cloudflare"]);
        // A Site or Param tenant names a param the account has.
        for c in &CONNECTORS {
            if let AccountTenant::Site(id) | AccountTenant::Param(id) = c.test.tenant {
                assert!(c.params.iter().any(|p| p.id == id && !p.secret), "{}", c.id);
            }
        }
        assert_eq!(row("linear").test.method, "POST");
        assert!(row("figma")
            .token_help_url
            .starts_with("https://developers.figma.com/"));
        assert!(row("figma").scopes_hint.contains("current_user:read"));
        assert!(row("github")
            .scopes_hint
            .contains("separate from the GitHub sign-in"));
    }

    #[test]
    fn every_allowlist_is_well_formed_and_passes_its_own_test_call() {
        for c in &CONNECTORS {
            assert_eq!(
                c.unlisted,
                format!("Bruhs can't call that endpoint of {} yet.", c.name)
            );
            assert!(!c.endpoints.is_empty() || c.graphql.is_some(), "{}", c.id);
            for endpoint in c.endpoints {
                assert!(!endpoint.methods.is_empty(), "{} {}", c.id, endpoint.path);
                assert!(
                    endpoint.methods.iter().all(|m| APP_METHODS.contains(m)),
                    "{} {}",
                    c.id,
                    endpoint.path
                );
                // A pattern with its wildcards filled in is a valid path.
                let filled = endpoint.path.replace("**", "x").replace('*', "x");
                assert!(validate_path(&filled).is_ok(), "{} {}", c.id, endpoint.path);
            }
            // The test call is itself an allowed read.
            let call = AppCall {
                app: c.id.into(),
                account: None,
                method: c.test.method.into(),
                path: c.test.path.into(),
                query: BTreeMap::new(),
                body: c.test.body.map(|body| serde_json::from_str(body).unwrap()),
            };
            assert_eq!(classify(c, &call), Ok(AppAction::Read), "{}", c.id);
        }
    }

    #[test]
    fn each_app_asks_for_the_values_its_accounts_need() {
        let ids = |id: &str| {
            connector(id)
                .unwrap()
                .params
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>()
        };
        assert_eq!(ids("trello"), ["token", "key"]);
        assert_eq!(ids("jira"), ["token", "email", "site"]);
        assert_eq!(ids("shopify"), ["token", "shop"]);
        assert_eq!(ids("github"), ["token"]);
        for c in &CONNECTORS {
            for p in c.params {
                assert_eq!(
                    p.secret,
                    matches!(p.id, "token" | "key"),
                    "{} {}",
                    c.id,
                    p.id
                );
            }
            if let AppHost::Account { param, .. } = c.host {
                assert!(
                    c.params.iter().any(|p| p.id == param && !p.secret),
                    "{}",
                    c.id
                );
            }
        }
        let shop = connector("shopify").unwrap();
        assert_eq!(
            base_url(shop, &BTreeMap::from([("shop".into(), " My-Shop ".into())])),
            Ok("https://my-shop.myshopify.com".into())
        );
        assert_eq!(base_url(shop, &BTreeMap::new()), Err(SITE_SHAPE));
        assert_eq!(
            base_url(connector("stripe").unwrap(), &BTreeMap::new()),
            Ok("https://api.stripe.com".into())
        );

        assert_eq!(validate_param(&TOKEN, ""), Err(ENTER_TOKEN));
        assert_eq!(validate_param(&KEY, "  "), Err(ENTER_TOKEN));
        assert_eq!(
            validate_param(&TOKEN, " ghp_abcdefgh\n"),
            Ok("ghp_abcdefgh".into())
        );
        assert_eq!(validate_param(&TOKEN, "short"), Err(TOKEN_SHAPE));
        assert_eq!(validate_param(&TOKEN, "abcd\nefgh"), Err(TOKEN_SHAPE));
        assert_eq!(validate_param(&TOKEN, "abcd\tefgh"), Err(TOKEN_SHAPE));
        assert_eq!(validate_param(&TOKEN, "abcd efgh"), Err(TOKEN_SHAPE));
        assert_eq!(validate_param(&TOKEN, "abcdéfghij"), Err(TOKEN_SHAPE));
        // M5: Trello quotes both values in its header, so neither may hold a
        // quote or a backslash to break out of it.
        for value in ["abcd\"efgh", "abcdefgh\", oauth_token=\"x", "abcd\\efgh"] {
            assert_eq!(validate_param(&TOKEN, value), Err(TOKEN_SHAPE), "{value}");
            assert_eq!(validate_param(&KEY, value), Err(TOKEN_SHAPE), "{value}");
        }
        assert_eq!(
            validate_param(&KEY, "0123456789abcdef"),
            Ok("0123456789abcdef".into())
        );
        assert_eq!(
            validate_param(&TOKEN, &"x".repeat(4096)).map(|t| t.len()),
            Ok(4096)
        );
        assert_eq!(validate_param(&TOKEN, &"x".repeat(4097)), Err(TOKEN_SHAPE));
        // M1: the copy names the minimum.
        assert!(TOKEN_SHAPE.contains("8 characters"));
        assert_eq!(
            validate_param(&EMAIL, " me@acme.test "),
            Ok("me@acme.test".into())
        );
        for bad in [
            "",
            "me",
            "@acme",
            "me@",
            "a@b@c",
            "me @acme.test",
            "me:x@acme.test",
        ] {
            assert_eq!(validate_param(&EMAIL, bad), Err(EMAIL_SHAPE), "{bad}");
        }
        assert_eq!(validate_param(&SITE, "Acme-2"), Ok("acme-2".into()));
        assert_eq!(validate_param(&SHOP, "acme-"), Err(SITE_SHAPE));
        assert!(!host_declared("-acme.atlassian.net") && !host_declared(".myshopify.com"));
        assert!(host_declared("API.GITHUB.COM"));
        assert_eq!(USER_AGENT, "bruhs-ai");
    }
}
