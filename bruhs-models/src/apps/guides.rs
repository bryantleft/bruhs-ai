//! Each connector's guide: model-facing text `list_apps` returns for one
//! app (spec: "a short skill text loaded on demand"). Each names the base
//! address, says Bruhs adds the token, lists common calls, how the app
//! pages results, and which calls need Send as me or Can deliver, and ends
//! "Other endpoints are refused." At most 800 bytes each, and every call a
//! guide names is on its app's allowlist.

pub(super) const GITHUB: &str = "Base address https://api.github.com. Bruhs adds the token and API version headers. Put query parameters in query, not the path. Common calls: GET /repos/{owner}/{repo}/issues (query state, labels), GET /repos/{owner}/{repo}/pulls, GET /repos/{owner}/{repo}/issues/{n}/comments, GET /search/issues (query q), POST /repos/{owner}/{repo}/issues {title, body}, POST /repos/{owner}/{repo}/issues/{n}/comments {body}. Paging: per_page up to 100 and page; the link header names the next page. Issues, comments and reviews post as the owner and need Send as me or approval. Opening and merging pull requests, writing contents or refs, and GraphQL mutations (POST /graphql) count as delivery and need Full access with Can deliver. Other endpoints are refused.";

pub(super) const NOTION: &str = "Base address https://api.notion.com. Bruhs adds the token and Notion-Version 2022-06-28. Common calls: POST /v1/search {query}, POST /v1/databases/{id}/query {filter}, GET /v1/pages/{id}, GET /v1/blocks/{id}/children, PATCH /v1/pages/{id} {properties}, POST /v1/pages {parent, properties}. Paging: page_size up to 100; pass next_cursor back as start_cursor while has_more is true. Search and database queries are reads. Comments (POST /v1/comments) post as the owner and need Send as me or approval. Page and block changes are writes. Other endpoints are refused.";

pub(super) const LINEAR: &str = "Base address https://api.linear.app. Bruhs adds the key. Everything is GraphQL: POST /graphql {query, variables}, with nothing in the URL query. Common queries: { viewer { id name } }, { issues(first: 50, filter: {...}) { nodes { identifier title state { name } } } }, { teams { nodes { id key } } }. Common mutations: issueCreate, issueUpdate, commentCreate. Paging: first and after, with pageInfo { hasNextPage endCursor }. Queries are reads. Every mutation is seen as the owner, so it needs Send as me or approval. Other endpoints are refused.";

pub(super) const AIRTABLE: &str = "Base address https://api.airtable.com. Bruhs adds the token. Common calls: GET /v0/meta/bases, GET /v0/meta/bases/{baseId}/tables, GET /v0/{baseId}/{table} (query filterByFormula, maxRecords), POST /v0/{baseId}/{table}/listRecords {filterByFormula}, POST /v0/{baseId}/{table} {records}, PATCH /v0/{baseId}/{table} {records}. Base ids start with app. Paging: pageSize up to 100; pass offset back while one is returned. Listing records is a read. Creating, updating and deleting records are writes. Other endpoints are refused.";

pub(super) const TODOIST: &str = "Base address https://api.todoist.com. Bruhs adds the token. Common calls: GET /api/v1/tasks (query project_id), GET /api/v1/projects, GET /api/v1/tasks/filter (query query), POST /api/v1/tasks {content, due_string}, POST /api/v1/tasks/{id} {content}, POST /api/v1/tasks/{id}/close. Paging: limit up to 200; pass next_cursor back as cursor while it is set. Comments (POST /api/v1/comments) and quick add (POST /api/v1/tasks/quick), whose note is a comment, post as the owner and need Send as me or approval. Other task changes are writes. Other endpoints are refused.";

pub(super) const TRELLO: &str = "Base address https://api.trello.com. Bruhs adds the key and token. Common calls: GET /1/members/me/boards, GET /1/boards/{id}/lists, GET /1/lists/{id}/cards, GET /1/cards/{id}, POST /1/cards (query idList, name, desc), PUT /1/cards/{id} (query idList, closed). Paging: most lists return everything; card actions take limit up to 1000 and before. Comments (POST /1/cards/{id}/actions/comments) post as the owner and need Send as me or approval. Card changes are writes. Other endpoints are refused.";

pub(super) const CLICKUP: &str = "Base address https://api.clickup.com. Bruhs adds the token. Common calls: GET /api/v2/team, GET /api/v2/team/{team_id}/space, GET /api/v2/space/{space_id}/folder, GET /api/v2/list/{list_id}/task (query page), POST /api/v2/list/{list_id}/task {name, description}, PUT /api/v2/task/{task_id} {status}. Paging: page from 0 while last_page is false, 100 tasks a page. Comments on tasks, lists and views and Chat messages (POST /api/v3/workspaces/{id}/chat/channels/{id}/messages) post as the owner and need Send as me or approval. Task changes are writes. Other endpoints are refused.";

pub(super) const HUBSPOT: &str = "Base address https://api.hubapi.com. Bruhs adds the token. Common calls: GET /crm/v3/objects/{type} (contacts, companies, deals, tickets; query properties, limit), GET /crm/v3/objects/{type}/{id}, POST /crm/v3/objects/{type}/search {filterGroups}, POST /crm/v3/objects/{type} {properties}, PATCH /crm/v3/objects/{type}/{id} {properties}. Paging: limit up to 100; pass paging.next.after back as after. Searches and batch reads are reads. Email (POST /marketing/v3/transactional/single-email/send) and conversation messages post as the owner and need Send as me or approval. Record changes are writes. Other endpoints are refused.";

pub(super) const JIRA: &str = "Base address https://{site}.atlassian.net, the site the owner connected. Bruhs adds the email and token. Use /rest/api/3 only. Common calls: POST /rest/api/3/search/jql {jql, fields, maxResults}, GET /rest/api/3/issue/{key}, GET /rest/api/3/project/search, GET /rest/api/3/issue/{key}/transitions, POST /rest/api/3/issue/{key}/transitions {transition}, PUT /rest/api/3/issue/{key} {fields}. Text fields use Atlassian Document Format. Paging: maxResults, with nextPageToken from search/jql. Searches are reads. Creating and editing issues, transitions, comments and notifications can post as the owner and need Send as me or approval. Other endpoints are refused.";

pub(super) const SHOPIFY: &str = "Base address https://{shop}.myshopify.com, the shop the owner connected. Bruhs adds the token. Use API version 2026-10. Common calls: GET /admin/api/2026-10/shop.json, GET /admin/api/2026-10/orders.json (query status, limit), GET /admin/api/2026-10/products.json, PUT /admin/api/2026-10/products/{id}.json {product}, POST /admin/api/2026-10/graphql.json {query}. Paging: limit up to 250; follow page_info from the link header. GraphQL queries are reads; every GraphQL mutation needs Send as me or approval. Orders, refunds, cancels, transactions, fulfillments, invoices and invites reach customers or move money and need Send as me or approval. Product changes are writes. Other endpoints are refused.";

pub(super) const STRIPE: &str = "Base address https://api.stripe.com. Bruhs adds the key. Stripe reads form fields, not JSON: pass every parameter in query, nested ones as metadata[key], and send no body. Common calls: GET /v1/customers (query email, limit), GET /v1/charges, GET /v1/payment_intents, GET /v1/invoices (query customer, status), GET /v1/balance, POST /v1/customers (query email, name). Paging: limit up to 100; pass the last id as starting_after while has_more is true. Charges, payments, captures, refunds, payouts, transfers, subscriptions, credit notes, invoices and invoice items can move money or email customers and need Send as me or approval. Customer, product and price changes are writes; a customer's balance is refused. Other endpoints are refused.";

pub(super) const CLOUDFLARE: &str = "Base address https://api.cloudflare.com. Bruhs adds the token. Common calls: GET /client/v4/zones (query name), GET /client/v4/zones/{zone_id}/dns_records (query type, name), POST /client/v4/zones/{zone_id}/dns_records {type, name, content, ttl}, PATCH /client/v4/zones/{zone_id}/dns_records/{id} {content}, DELETE /client/v4/zones/{zone_id}/dns_records/{id}, GET /client/v4/accounts. Paging: page and per_page; result_info gives total_pages. DNS changes are writes. Other endpoints are refused.";

pub(super) const SLACK: &str = "Base address https://slack.com. Bruhs adds the token. Methods live under /api/{method}; send reads as GET with query, writes as POST with a JSON body. Common calls: GET /api/conversations.list (query types, limit), GET /api/conversations.history (query channel, limit), GET /api/conversations.replies (query channel, ts), GET /api/users.info (query user), GET /api/search.messages (query query), POST /api/chat.postMessage {channel, text}. Check ok in every answer. Paging: limit, then pass response_metadata.next_cursor back as cursor. Posting, editing, scheduling and reactions act as the owner and need Send as me or approval. Other endpoints are refused.";

pub(super) const FIGMA: &str = "Base address https://api.figma.com. Bruhs adds the token. Common calls: GET /v1/me, GET /v1/files/{key} (query depth, ids), GET /v1/files/{key}/nodes (query ids), GET /v1/images/{key} (query ids, format), GET /v1/files/{key}/comments, GET /v1/projects/{id}/files. Paging: most reads return everything; ask for less with depth or ids. Comments and reactions (POST /v1/files/{key}/comments) post as the owner and need Send as me or approval. Other endpoints are refused.";

#[cfg(test)]
mod tests {
    use super::super::{classify, AppAction, AppCall, AppHost, APP_METHODS, CONNECTORS};

    /// Every guide's last sentence (ruling A).
    const REFUSED_LINE: &str = "Other endpoints are refused.";
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn each_guide_names_its_address_paging_and_what_asks() {
        for connector in &CONNECTORS {
            let base = match connector.host {
                AppHost::Fixed(host) => format!("https://{host}"),
                AppHost::Account { param, suffix } => format!("https://{{{param}}}{suffix}"),
            };
            let guide = connector.guide;
            let id = connector.id;
            assert!(guide.starts_with(&format!("Base address {base}")), "{id}");
            assert!(
                guide.contains("Bruhs adds the") && guide.contains("Paging:"),
                "{id}"
            );
            assert!(guide.ends_with(REFUSED_LINE), "{id}");
            assert!(guide.len() <= 800, "{id} is {} bytes", guide.len());
            let classes: Vec<AppAction> = connector
                .endpoints
                .iter()
                .map(|endpoint| endpoint.action)
                .chain(connector.graphql.map(|graphql| graphql.mutation))
                .collect();
            let sends = classes.contains(&AppAction::SendAsOwner);
            assert_eq!(guide.contains("Send as me or approval"), sends, "{id}");
            let delivers = classes.contains(&AppAction::Delivery);
            assert_eq!(guide.contains("count as delivery"), delivers, "{id}");
        }
        let shopify = CONNECTORS.iter().find(|c| c.id == "shopify").unwrap();
        assert!(shopify.test.path.starts_with("/admin/api/2026-10/"));
        assert!(shopify.guide.contains("2026-10") && !shopify.guide.contains("2025-"));
    }

    /// Every "METHOD /path" a guide teaches is on its app's allowlist, with
    /// `{placeholders}` filled in (with `app1`, which also fits Airtable's base
    /// ids).
    #[test]
    fn every_call_a_guide_names_is_allowed() {
        for connector in &CONNECTORS {
            let words: Vec<&str> = connector.guide.split_whitespace().collect();
            let mut named = 0;
            for pair in words.windows(2) {
                let [method, path] = pair else { continue };
                if !APP_METHODS.contains(method) || !path.starts_with('/') {
                    continue;
                }
                let path = path.trim_end_matches([',', '.', ')']);
                let filled: String = path
                    .split('/')
                    .map(|segment| match segment.find('{') {
                        Some(open) => {
                            let close = segment.rfind('}').map_or(segment.len(), |close| close + 1);
                            format!("{}app1{}", &segment[..open], &segment[close..])
                        }
                        None => segment.to_string(),
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                let body =
                    (!matches!(*method, "GET" | "DELETE")).then(|| json!({"query": "{ a }"}));
                let call = AppCall {
                    app: connector.id.into(),
                    account: None,
                    method: (*method).into(),
                    path: filled.clone(),
                    query: BTreeMap::new(),
                    body,
                };
                assert!(
                    classify(connector, &call).is_ok(),
                    "{} {method} {filled}",
                    connector.id
                );
                named += 1;
            }
            assert!(
                named >= 3 || connector.id == "linear",
                "{} names {named} calls",
                connector.id
            );
        }
    }
}
