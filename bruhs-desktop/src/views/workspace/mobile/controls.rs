use super::*;
use bruhs_ui::press::on_press_view;
use bruhs_ui::zoom::px;
fn note(text: impl Into<SharedString>) -> Div {
    div()
        .min_w_0()
        .text_size(px(12.0))
        .text_color(theme::rambutan_400())
        .child(text.into())
}
fn field(label: &'static str, input: &Entity<InputState>, disabled: bool) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .min_w_0()
        .child(note(label))
        .child(Input::new(input).aria_label(label).disabled(disabled))
}
fn control(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    disabled: bool,
) -> Stateful<Div> {
    let label = label.into();
    web_button(
        id,
        WebButtonVariant::Outline,
        WebButtonSize::Small,
        ButtonState::disabled_if(disabled),
    )
    .aria_label(label.clone())
    .child(label)
}
trait Activate {
    fn activate(
        self,
        cx: &Context<'_, Workspace>,
        f: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
    ) -> Self;
}
impl Activate for Stateful<Div> {
    fn activate(
        self,
        cx: &Context<'_, Workspace>,
        f: impl Fn(&mut Workspace, &mut Window, &mut Context<'_, Workspace>) + 'static,
    ) -> Self {
        on_press_view(self, cx, f)
    }
}
impl Workspace {
    pub(in crate::views::workspace) fn select_access_option(
        &mut self,
        id: &str,
        value: &str,
        cx: &mut Context<'_, Self>,
    ) -> bool {
        if id != "settings-environment-icon" {
            return false;
        }
        if AccessPreferences::ICONS.iter().any(|(k, _)| *k == value) {
            self.access_preferences.environment_icon = value.to_owned();
            self.persist_and_notify(cx);
        }
        true
    }

    pub(in crate::views::workspace) fn render_access_identity_card(
        &self,
        cx: &Context<'_, Self>,
    ) -> AnyElement {
        let tailscale_origin = self.mobile.origin.read(cx).value().trim().to_owned();
        let tailscale_configured = tailscale_origin.contains(".ts.net");
        let tailscale_action = control(
            "access-tailscale-template",
            if tailscale_configured {
                "Using Tailscale"
            } else {
                "Use Tailscale Serve"
            },
            tailscale_configured || !matches!(self.mobile.connection, Connection::Off),
        )
        .activate(cx, |this, window, cx| {
            this.mobile.proxy = true;
            this.mobile.origin.update(cx, |input, cx| {
                input.set_value("https://this-machine.tailnet.ts.net", window, cx)
            });
            cx.notify();
        });
        let rows = vec![
            self.settings_row(
                "settings-access-environment-icon",
                "Environment icon",
                "Glyph shown beside this machine in environment lists and linked sidebars.",
                settings_ui::control_column(self.choice_control(
                    "settings-environment-icon",
                    "Environment icon",
                    &self.access_preferences.environment_icon,
                    AccessPreferences::ICONS,
                    cx,
                )),
            ),
            self.settings_row(
                "settings-access-require-approval",
                "Require approval for new devices",
                "The device has to be approved here before it can connect again.",
                settings_ui::switch(
                    "settings-toggle-require-device-approval",
                    self.access_preferences.require_device_approval,
                    false,
                    "Require approval for new devices",
                    cx,
                    |this, _, cx| {
                        this.access_preferences.require_device_approval =
                            !this.access_preferences.require_device_approval;
                        this.persist_and_notify(cx);
                    },
                ),
            ),
            self.settings_row(
                "settings-access-hold-webhooks",
                "Hold webhooks while offline",
                "Keep webhook requests for up to 24 hours while this environment is offline.",
                settings_ui::switch(
                    "settings-toggle-hold-webhooks-offline",
                    self.access_preferences.hold_webhooks_while_offline,
                    false,
                    "Hold webhooks while offline",
                    cx,
                    |this, _, cx| {
                        this.access_preferences.hold_webhooks_while_offline =
                            !this.access_preferences.hold_webhooks_while_offline;
                        this.persist_and_notify(cx);
                    },
                ),
            ),
            self.settings_row(
                "settings-access-tailscale",
                "Tailscale HTTPS",
                if tailscale_configured {
                    format!("Configured with {tailscale_origin} over loopback proxy.")
                } else {
                    "Expose phone access over your tailnet using Tailscale Serve and MagicDNS."
                        .into()
                },
                tailscale_action.into_any_element(),
            ),
        ];
        div()
            .mb_3()
            .child(self.settings_card("This machine", "", rows))
            .into_any_element()
    }

    pub(in crate::views::workspace) fn render_mobile(
        &self,
        cx: &Context<'_, Self>,
    ) -> Stateful<Div> {
        let off = matches!(self.mobile.connection, Connection::Off);
        let offering = self.mobile.offer.is_some();
        let mut panel = div()
            .id("desktop-mobile-access")
            .debug_selector(|| "desktop-mobile-access".into())
            .mb_3()
            .p_3()
            .rounded(px(8.0))
            .border_1()
            .border_color(theme::longan_800())
            .bg(theme::longan_900())
            .flex()
            .flex_col()
            .gap_3()
            .child(
                div()
                    .text_size(px(15.0))
                    .text_color(theme::rambutan_100())
                    .child("Phone access"),
            )
            .child(note(
                "Read project chats and live output from your phone while bruhs is running.",
            ));
        if off {
            panel=panel.child(note("Use an HTTPS origin your phone trusts, such as Tailscale Serve or trusted TLS PEM files."))
                .child(field("HTTPS origin",&self.mobile.origin,false)).child(field("Listen IP:port",&self.mobile.bind,false))
                .child(control("mobile-mode",if self.mobile.proxy{"TLS: existing HTTPS proxy (loopback only)"}else{"TLS: trusted certificate and key"},false).activate(cx,|this,_,cx|{this.mobile.proxy = !this.mobile.proxy;cx.notify();}));
            if !self.mobile.proxy {
                panel = panel
                    .child(field(
                        "Certificate PEM path",
                        &self.mobile.certificate,
                        false,
                    ))
                    .child(field("Private key PEM path", &self.mobile.key, false));
            }
            panel = panel.child(
                control("mobile-start", "Start phone access", false)
                    .debug_selector(|| "mobile-start".into())
                    .activate(cx, |this, _, cx| this.start_mobile(cx)),
            );
        } else if let Some(host) = self.mobile.host() {
            panel = panel
                .child(note(format!(
                    "Listening at {}, backend {}",
                    host.origin(),
                    host.address()
                )))
                .child(note(
                    "Each paired device has its own scope, expiry, and revoke control.",
                ));
            let mut scopes_row = div()
                .flex()
                .flex_wrap()
                .gap_2()
                .child(note("Read chats (required)"));
            for (scope, label) in [
                (MobileScope::Prompt, "Send prompts"),
                (MobileScope::Approve, "Approve tools"),
                (MobileScope::Diff, "Review diffs and linked project files"),
                (MobileScope::LinkedDrafts, "Open linked drafts"),
            ] {
                scopes_row = scopes_row.child(
                    control(
                        SharedString::from(format!("mobile-scope-{scope:?}")),
                        format!(
                            "{} {label}",
                            if self.mobile.scopes.contains(&scope) {
                                "✓"
                            } else {
                                "○"
                            }
                        ),
                        offering,
                    )
                    .activate(cx, move |this, _, cx| {
                        if this.mobile.offer.is_some() {
                            return;
                        }
                        if !this.mobile.scopes.remove(&scope) {
                            this.mobile.scopes.insert(scope);
                        }
                        cx.notify();
                    }),
                );
            }
            panel = panel
                .child(scopes_row)
                .when(self.mobile.scopes.contains(&MobileScope::LinkedDrafts), |panel| {
                    panel.child(note("Linked draft access can confirm repository clones and pull request checkouts that run saved project setup."))
                })
                .child(field(
                    "Device lifetime in hours (1 to 24)",
                    &self.mobile.hours,
                    offering,
                ))
                .child(
                    div()
                        .flex()
                        .flex_wrap()
                        .gap_2()
                        .child(
                            control("mobile-pair", "Show pairing QR", false)
                                .debug_selector(|| "mobile-pair".into())
                                .activate(cx, |this, _, cx| this.offer_mobile(cx)),
                        )
                        .child(
                            control("mobile-stop", "Stop phone access", false)
                                .debug_selector(|| "mobile-stop".into())
                                .activate(cx, |this, _, cx| this.stop_mobile(cx)),
                        ),
                );
            if let Some((_, qr)) = &self.mobile.offer {
                let qr = qr.clone();
                let side = (qr.size + 8).to_f32_lossy() * 4.0;
                panel=panel.child(note("Scan now. This one-use QR expires in five minutes. Tokens expire at the selected lifetime or when the desktop restarts."))
                    .child(canvas(|_,_,_|{},move|bounds,(),window,_|{
                        window.paint_quad(fill(bounds,rgb(0xffffff)));
                        for y in 0..qr.size {for x in 0..qr.size {if qr.modules[y*qr.size+x] {
                            let origin=point(bounds.origin.x+px((x+4).to_f32_lossy()*4.0),bounds.origin.y+px((y+4).to_f32_lossy()*4.0));
                            window.paint_quad(fill(Bounds::new(origin,size(px(4.0),px(4.0))),rgb(0x000000)));
                        }}}
                    }).size(px(side)).flex_none())
                    .child(div().flex().flex_wrap().gap_2()
                        .child(control("mobile-copy","Copy pairing link",false).activate(cx,|this,_,cx|{if let Some((offer,_))=&this.mobile.offer{cx.write_to_clipboard(ClipboardItem::new_string(offer.url.to_string()));}}))
                        .child(control("mobile-close-qr","Close QR",false).activate(cx,|this,_,cx|{this.close_mobile_offer();cx.notify();})));
            }
            match host.authority().devices() {
                Ok(devices) => {
                    panel = panel.child(note("Paired devices (restart requires pairing again)"));
                    if devices.is_empty() {
                        panel = panel.child(note("No devices paired."));
                    }
                    for device in devices {
                        let expired = device.expires_at_ms <= chrono::Utc::now().timestamp_millis();
                        let last = device
                            .last_seen_ms
                            .and_then(chrono::DateTime::from_timestamp_millis)
                            .map_or_else(
                                || "never".into(),
                                |time| time.format("%Y-%m-%d %H:%M UTC").to_string(),
                            );
                        let expiry = chrono::DateTime::from_timestamp_millis(device.expires_at_ms)
                            .map_or_else(
                                || "unknown".into(),
                                |time| time.format("%Y-%m-%d %H:%M UTC").to_string(),
                            );
                        let state = if device.revoked {
                            "revoked"
                        } else if expired {
                            "expired"
                        } else {
                            "active"
                        };
                        panel = panel.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap_1()
                                .border_t_1()
                                .border_color(theme::longan_800())
                                .pt_2()
                                .child(note(format!(
                                    "{}, {state}, {:?}",
                                    device.name, device.scopes
                                )))
                                .child(note(format!("Last seen {last}, expires {expiry}")))
                                .child(
                                    control(
                                        SharedString::from(format!("mobile-revoke-{}", device.id)),
                                        "Revoke device",
                                        device.revoked,
                                    )
                                    .activate(cx, move |this, _, cx| {
                                        this.revoke_mobile(device.id, cx)
                                    }),
                                ),
                        );
                    }
                }
                Err(error) => panel = panel.child(note(error.to_string())),
            }
        } else {
            panel = panel.child(note(
                if matches!(self.mobile.connection, Connection::Starting(_)) {
                    "Starting phone access…"
                } else {
                    "Stopping phone access…"
                },
            ));
        }
        if let Some(error) = &self.mobile.error {
            panel = panel.child(note(error.clone()).text_color(theme::lychee_300()));
        }
        panel
    }
}
