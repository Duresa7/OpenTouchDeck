//! Integrations, fed from the Companion's `IntegrationsSession`. The session holds every rule:
//! the switch, setup, one Save for settings and secrets, Forget and checks. This module only maps
//! its view to the `Integrations` global and sends the page's callbacks back as operations.
use super::{Screens, sync};
use crate::{
    AppWindow, DeliveryProblem, IntegrationField, IntegrationPage, IntegrationRow,
    IntegrationSecret, Integrations, TableCell, UsageAccount,
};
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{rc::Rc, time::SystemTime};
use td_companion::integrations::session::{
    Busy, DetailView, FieldKind, FieldValue, IntegrationsSession, Load, NoticeKind, Outcome, Tone,
};

/// The pages in the Icons group. They have pages of their own, not the schema's.
pub(super) const ICON_PACKS: &str = "icon-packs";
pub(super) const BRANDFETCH: &str = "brandfetch";

/// Where this PC keeps secrets, as the secret's hint names it.
const KEYCHAIN: &str = if cfg!(windows) {
    "Windows Credential Manager"
} else {
    "your keyring"
};

fn tone(tone: Option<Tone>) -> &'static str {
    match tone {
        Some(Tone::Ok) => "ok",
        Some(Tone::Bad) => "bad",
        Some(Tone::Wait) => "wait",
        None => "",
    }
}

/// The letter that stands in for an Integration's icon.
fn monogram(name: &str) -> String {
    name.chars()
        .next()
        .map(|c| c.to_uppercase().collect())
        .unwrap_or_default()
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

/// A count with digit grouping: 1,284,567.
fn count(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// An account's status as a word: "active" → "Active", "rate_limited" → "Rate limited".
fn badge(status: &str) -> String {
    let words = status.trim().replace('_', " ").to_lowercase();
    let mut chars = words.chars();
    chars.next().map_or_else(String::new, |first| {
        first.to_uppercase().chain(chars).collect()
    })
}

/// When a limit resets: "Resets in 3 hr", "Resets now", or "Reset time unknown".
fn resets(at_ms: Option<i64>, now_ms: u64) -> String {
    let Some(at) = at_ms else {
        return "Reset time unknown".into();
    };
    let left = at.saturating_sub(now_ms as i64);
    if left <= 0 {
        return "Resets now".into();
    }
    let minutes = (left as f64 / 60_000.0).round() as i64;
    if minutes < 90 {
        format!("Resets in {} min", minutes.max(1))
    } else if minutes < 36 * 60 {
        format!("Resets in {} hr", (minutes as f64 / 60.0).round() as i64)
    } else {
        format!(
            "Resets in {} days",
            (minutes as f64 / 1440.0).round() as i64
        )
    }
}

fn kind(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Boolean => "boolean",
        FieldKind::Integer => "integer",
        FieldKind::Number => "number",
        FieldKind::Text => "text",
        FieldKind::Json => "json",
    }
}

impl Screens {
    pub(super) fn wire_integrations(self: &Rc<Self>, ui: &AppWindow) {
        let page = ui.global::<Integrations>();
        page.set_rows(ModelRc::from(self.models.integrations.clone()));
        page.set_fields(ModelRc::from(self.models.fields.clone()));
        page.set_secrets(ModelRc::from(self.models.secrets.clone()));
        page.set_deliveries(ModelRc::from(self.models.deliveries.clone()));
        page.set_accounts(ModelRc::from(self.models.accounts.clone()));
        on!(self, page, on_flip, |s, id: SharedString, on: bool| {
            let session = s.integrations.clone();
            s.spawn(async move {
                session.flip(&id, on).await;
            });
        });
        on!(self, page, on_open, |s, id: SharedString| s
            .open_integration(&id));
        on!(self, page, on_close, |s| s.close_integration());
        on!(
            self,
            page,
            on_set_text,
            |s, name: SharedString, text: SharedString| {
                s.integrations
                    .set_value(&name, FieldValue::Text(text.into()));
            }
        );
        on!(
            self,
            page,
            on_set_flag,
            |s, name: SharedString, on: bool| {
                s.integrations.set_value(&name, FieldValue::Bool(on));
            }
        );
        on!(
            self,
            page,
            on_type_secret,
            |s, name: SharedString, text: SharedString| {
                s.integrations.enter_secret(&name, &text);
            }
        );
        on!(self, page, on_replace_secret, |s, name: SharedString| {
            s.replacing.borrow_mut().insert(name.into());
            s.refresh();
        });
        on!(self, page, on_keep_secret, |s, name: SharedString| {
            s.replacing.borrow_mut().remove(name.as_str());
            s.integrations.enter_secret(&name, "");
            s.refresh();
        });
        on!(self, page, on_forget_secret, |s, name: SharedString| {
            let session = s.integrations.clone();
            s.spawn(async move {
                session.forget_secret(&name).await;
            });
        });
        on!(self, page, on_save, |s| s.save_integration());
        on!(self, page, on_check, |s| {
            let session = s.integrations.clone();
            s.spawn(async move {
                session.refresh().await;
            });
        });
        on!(self, page, on_show_approval, |s| {
            let session = s.integrations.clone();
            s.spawn(async move {
                session.show_prompt().await;
            });
        });
        // A page asked for before the window was wired, such as a mock screenshot's.
        let wanted = page.get_page_id();
        if !wanted.is_empty() {
            self.open_integration(&wanted);
        }
    }

    /// The window's Integrations session, started inside the runtime so it follows the
    /// Companion's changes there: outside it, a status change would mark the open page as
    /// reloading and schedule nothing.
    pub(super) fn integrations_session(
        handle: &td_companion::companion::Handle,
        runtime: &tokio::runtime::Handle,
    ) -> IntegrationsSession {
        let _inside = runtime.enter();
        IntegrationsSession::new(handle.clone())
    }

    fn open_integration(&self, id: &str) {
        self.replacing.borrow_mut().clear();
        if let Some(ui) = self.ui.upgrade() {
            ui.global::<Integrations>()
                .set_confirming_forget(SharedString::new());
        }
        if id == ICON_PACKS || id == BRANDFETCH {
            self.integrations.close();
            if let Some(ui) = self.ui.upgrade() {
                ui.global::<Integrations>().set_page_id(id.into());
            }
            self.refresh();
            return;
        }
        let session = self.integrations.clone();
        let id = id.to_owned();
        self.spawn(async move {
            session.open(&id).await;
        });
    }

    fn close_integration(&self) {
        self.integrations.close();
        self.replacing.borrow_mut().clear();
        if let Some(ui) = self.ui.upgrade() {
            let page = ui.global::<Integrations>();
            page.set_page_id(SharedString::new());
            page.set_confirming_forget(SharedString::new());
        }
        self.refresh();
    }

    fn save_integration(&self) {
        // A secret typed for this Save shows "Saved" again once it's stored; one whose Replace
        // was chosen but left empty keeps its field.
        if let Some(detail) = self.integrations.view().detail {
            self.replacing.borrow_mut().retain(|name| {
                !detail
                    .secrets
                    .iter()
                    .any(|s| &s.field.name == name && s.entered)
            });
        }
        let session = self.integrations.clone();
        self.spawn(async move {
            session.save().await;
        });
    }

    pub(super) fn refresh_integrations(&self, ui: &AppWindow) {
        let view = self.integrations.view();
        let page = ui.global::<Integrations>();
        sync(
            &self.models.integrations,
            view.rows
                .iter()
                .map(|row| {
                    // The line carries the status words only while something is wrong.
                    let problem = row.line == row.status.words;
                    IntegrationRow {
                        id: row.id.as_str().into(),
                        name: row.name.as_str().into(),
                        monogram: monogram(&row.name).into(),
                        line: row.line.as_str().into(),
                        tone: if problem { tone(row.status.tone) } else { "" }.into(),
                        checked: row.switch.checked,
                        locked: row.switch.locked,
                        settings: row.settings,
                    }
                })
                .collect(),
        );
        let notice = view.notice.as_ref();
        page.set_notice(notice.map_or("", |n| n.text.as_str()).into());
        page.set_notice_detail(
            notice
                .and_then(|n| n.detail.as_deref())
                .unwrap_or_default()
                .into(),
        );

        // The page follows the session: its open page, or the list once it closes.
        let shown = page.get_page_id();
        match &view.detail {
            Some(detail) => {
                if shown != detail.id.as_str() {
                    page.set_page_id(detail.id.as_str().into());
                    page.set_confirming_forget(SharedString::new());
                }
                self.show_detail(&page, detail);
            }
            None => {
                if shown != ICON_PACKS && shown != BRANDFETCH && !shown.is_empty() {
                    page.set_page_id(SharedString::new());
                }
                sync(&self.models.fields, Vec::new());
                sync(&self.models.secrets, Vec::new());
                sync(&self.models.deliveries, Vec::new());
                sync(&self.models.accounts, Vec::new());
            }
        }
    }

    fn show_detail(&self, page: &Integrations<'_>, detail: &DetailView) {
        let now = now_ms();
        let notice = detail.notice.as_ref();
        let refresh = detail.refresh.as_ref();
        let usage = detail.usage.as_ref();
        page.set_page(IntegrationPage {
            id: detail.id.as_str().into(),
            name: detail.name.as_str().into(),
            monogram: monogram(&detail.name).into(),
            line: detail.line.into(),
            load: match detail.load {
                Load::Loading => "loading",
                Load::Ready => "ready",
                Load::Failed => "failed",
            }
            .into(),
            header: detail.header_line(now).into(),
            tone: tone(detail.status.tone).into(),
            status_detail: detail.status.detail.clone().unwrap_or_default().into(),
            checked: detail.switch.checked,
            switch_locked: detail.switch.locked,
            settings_title: detail.settings_title.into(),
            has_form: !detail.fields.is_empty() || !detail.secrets.is_empty(),
            dirty: detail.dirty(),
            can_save: detail.can_save(),
            saving: detail.busy == Some(Busy::Save),
            keychain: KEYCHAIN.into(),
            has_refresh: refresh.is_some(),
            refresh_usable: refresh.is_some_and(|r| r.usable),
            refresh_line: refresh.map_or("", |r| r.line.as_str()).into(),
            checking: detail.busy == Some(Busy::Refresh),
            approval: detail
                .approval
                .as_ref()
                .map(|a| {
                    std::iter::once(a.program.as_str())
                        .chain(a.args.iter().map(String::as_str))
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default()
                .into(),
            notice: notice.map_or("", |n| n.text.as_str()).into(),
            notice_kind: match notice.map(|n| n.kind) {
                Some(NoticeKind::Error) => "error",
                Some(NoticeKind::Ok) => "ok",
                _ => "info",
            }
            .into(),
            notice_detail: notice
                .and_then(|n| n.detail.as_deref())
                .unwrap_or_default()
                .into(),
            notice_at: match notice.map(|n| n.at()) {
                Some(Outcome::Settings) => "settings",
                Some(Outcome::Secret) => "secret",
                Some(Outcome::Status) => "status",
                Some(Outcome::Page) | None => "page",
            }
            .into(),
        });
        sync(
            &self.models.fields,
            detail
                .fields
                .iter()
                .map(|field| {
                    let value = detail.values.get(&field.name);
                    IntegrationField {
                        name: field.name.as_str().into(),
                        kind: kind(field.kind).into(),
                        label: field.label.as_str().into(),
                        placeholder: field.placeholder.unwrap_or_default().into(),
                        unit: field.unit.unwrap_or_default().into(),
                        value: match value {
                            Some(FieldValue::Text(text)) => text.as_str().into(),
                            _ => SharedString::new(),
                        },
                        on: value == Some(&FieldValue::Bool(true)),
                        error: detail
                            .errors
                            .get(&field.name)
                            .map_or("", String::as_str)
                            .into(),
                        warning: detail.warning(&field.name).unwrap_or_default().into(),
                    }
                })
                .collect(),
        );
        let replacing = self.replacing.borrow();
        sync(
            &self.models.secrets,
            detail
                .secrets
                .iter()
                .map(|secret| IntegrationSecret {
                    name: secret.field.name.as_str().into(),
                    label: secret.field.label.as_str().into(),
                    set: secret.set,
                    typing: !secret.set || secret.entered || replacing.contains(&secret.field.name),
                    error: detail
                        .errors
                        .get(&secret.field.name)
                        .map_or("", String::as_str)
                        .into(),
                })
                .collect(),
        );
        sync(
            &self.models.deliveries,
            detail
                .deliveries
                .iter()
                .map(|d| DeliveryProblem {
                    text: d.text.as_str().into(),
                    detail: d.detail.as_str().into(),
                })
                .collect(),
        );
        let accounts = usage.map(|u| u.accounts.as_slice()).unwrap_or_default();
        sync(
            &self.models.accounts,
            accounts
                .iter()
                .map(|account| {
                    let limit = |window: Option<(f64, Option<i64>)>| match window {
                        Some((pct, at)) => (
                            format!("{}%", count(pct.round().max(0.0) as u64)),
                            resets(at, now),
                        ),
                        None => ("—".to_owned(), String::new()),
                    };
                    let five = limit(account.five_hour.as_ref().map(|w| (w.pct, w.resets_at_ms)));
                    let seven = limit(account.seven_day.as_ref().map(|w| (w.pct, w.resets_at_ms)));
                    let counts = format!(
                        "{} OK · {} failed · {} in 10 min",
                        count(account.success),
                        count(account.failed),
                        count(account.recent_10m)
                    );
                    let cell = |label: &str, value: String, hint: String| TableCell {
                        label: label.into(),
                        value: value.into(),
                        hint: hint.into(),
                    };
                    UsageAccount {
                        title: account.account.as_str().into(),
                        cells: ModelRc::new(VecModel::from(vec![
                            cell("Service", account.provider.clone(), String::new()),
                            cell(
                                "Plan",
                                account.plan.clone().unwrap_or_else(|| "—".into()),
                                String::new(),
                            ),
                            cell("5-hour limit", five.0, five.1),
                            cell("7-day limit", seven.0, seven.1),
                            cell(
                                "Status",
                                badge(&account.status),
                                match &account.error {
                                    Some(error) => format!("{error} · {counts}"),
                                    None => counts,
                                },
                            ),
                        ])),
                    }
                })
                .collect(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_group_their_digits() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1_284_567), "1,284,567");
    }

    #[test]
    fn statuses_read_as_words() {
        assert_eq!(badge("active"), "Active");
        assert_eq!(badge("RATE_LIMITED"), "Rate limited");
        assert_eq!(badge(""), "");
    }

    #[test]
    fn resets_say_how_long_is_left() {
        let now = 1_000_000_000;
        let at = |minutes: i64| Some(now as i64 + minutes * 60_000);
        assert_eq!(resets(None, now), "Reset time unknown");
        assert_eq!(resets(at(0), now), "Resets now");
        assert_eq!(resets(at(-5), now), "Resets now");
        assert_eq!(resets(Some(now as i64 + 1_000), now), "Resets in 1 min");
        assert_eq!(resets(at(45), now), "Resets in 45 min");
        assert_eq!(resets(at(180), now), "Resets in 3 hr");
        assert_eq!(resets(at(2 * 1440), now), "Resets in 2 days");
    }

    #[test]
    fn the_monogram_is_the_first_letter() {
        assert_eq!(monogram("this PC"), "T");
        assert_eq!(monogram(""), "");
    }
}
