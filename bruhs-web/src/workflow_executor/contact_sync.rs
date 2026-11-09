use futures_util::{stream, StreamExt};

use super::*;
use crate::components::banner::BannerKind;

impl WorkflowExecutor {
    pub(super) fn report_people_cache(self, message: String) {
        self.people_cache_error.set(Some(message.clone()));
        self.banners.push(BannerKind::Error, message, None);
    }

    pub fn retry_contact_sync(self) {
        if self.contact_syncing.get_untracked() || self.contact_merging.get_untracked() {
            self.contact_sync_again.set(true);
            return;
        }
        self.contact_syncing.set(true);
        self.contact_sync_error.set(None);
        wasm_bindgen_futures::spawn_local(async move {
            match storage::people::outbox::pending().await {
                Ok(pending) => {
                    stream::iter(pending).map(|person| async move {
                        match crate::api::create_person(&person).await {
                            Ok(reply) => match storage::people::outbox::acknowledge(&person, &reply).await {
                                Ok(()) => self.cache_people(vec![reply]).await,
                                Err(error) => self.contact_sync_error.set(Some(error.to_string())),
                            },
                            Err(_) => self.contact_sync_error.set(Some("Contact sync is pending. Contacts remain saved locally; retry when the server is connected.".into())),
                        }
                    }).buffer_unordered(4).for_each(|()| std::future::ready(())).await;
                }
                Err(error) => self.report_people_cache(format!(
                    "Contact sync queue could not load. No contacts were sent. {error}"
                )),
            }
            self.contact_syncing.set(false);
            if self.contact_sync_again.get_untracked() {
                self.contact_sync_again.set(false);
                self.retry_contact_sync();
            }
        });
    }
}
