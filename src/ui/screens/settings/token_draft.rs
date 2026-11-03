use eframe::egui;

/// Credential edits live only in memory until the Settings Save action commits them.
/// Intentionally does not implement Debug or serialization.
#[derive(Default)]
pub struct TokenDraft {
    pub input: String,
    pending: Option<TokenChange>,
}

#[derive(Clone)]
pub enum TokenChange {
    Set(String),
    Remove,
}

/// A persisted credential can still be rejected or unverifiable by GitHub.
pub fn verification_allows_close(status: craft_apps_manager::network::TokenStatus) -> bool {
    use craft_apps_manager::network::TokenStatus;
    matches!(status, TokenStatus::Connected | TokenStatus::NotConfigured)
}

pub(super) fn prepare_token_paste(ctx: &egui::Context, field: egui::Id) {
    if !ctx.memory(|memory| memory.has_focus(field)) {
        return;
    }
    // Some VM clipboards include the C-string terminator in their text payload.
    // Normalize only pasted trailing NULs; credential validation handles all else.
    ctx.input_mut(|input| {
        for event in &mut input.events {
            if let egui::Event::Paste(text) = event {
                text.truncate(text.trim_end_matches('\0').len());
            }
        }
    });
}

impl TokenDraft {
    pub fn row_action_label(&self) -> &'static str {
        match self.pending {
            Some(_) => "Undo",
            None => "Remove",
        }
    }

    pub fn cancel_pending_set(&mut self) -> bool {
        if matches!(self.pending, Some(TokenChange::Set(_))) {
            self.discard();
            true
        } else {
            false
        }
    }

    pub fn undo_removal(&mut self) -> bool {
        if matches!(self.pending, Some(TokenChange::Remove)) {
            self.pending = None;
            true
        } else {
            false
        }
    }

    pub fn row(&self, saved: bool) -> Option<(&'static str, &'static str)> {
        match &self.pending {
            Some(TokenChange::Set(_)) if saved => Some((
                "Replacement pending",
                "Your saved token stays active until Save",
            )),
            Some(TokenChange::Set(_)) => Some(("New token pending", "Click Save to apply")),
            Some(TokenChange::Remove) => Some(("Token removal pending", "Click Save to apply")),
            None if saved => Some(("Saved token", "System credential store")),
            None => None,
        }
    }
    pub fn has_pending(&self) -> bool {
        self.pending.is_some() || !self.input.trim().is_empty()
    }

    pub fn stage(&mut self) {
        if !self.input.trim().is_empty() {
            self.pending = Some(TokenChange::Set(self.input.trim().to_owned()));
        }
        self.input.clear();
    }

    pub fn remove(&mut self) {
        self.input.clear();
        self.pending = Some(TokenChange::Remove);
    }

    pub fn change(&self) -> Option<TokenChange> {
        if self.input.trim().is_empty() {
            self.pending.clone()
        } else {
            Some(TokenChange::Set(self.input.trim().to_owned()))
        }
    }

    pub fn discard(&mut self) {
        self.input.clear();
        self.pending = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit_event(event: egui::Event, token_focused: bool) -> (String, String) {
        let ctx = egui::Context::default();
        let token_id = egui::Id::new("token-paste-test");
        let other_id = egui::Id::new("other-paste-test");
        let mut token = String::new();
        let mut other = String::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                ui.add(egui::TextEdit::singleline(&mut token).id(token_id));
                ui.add(egui::TextEdit::singleline(&mut other).id(other_id));
                ctx.memory_mut(|memory| {
                    memory.request_focus(if token_focused { token_id } else { other_id });
                });
            });
        });
        let input = egui::RawInput {
            events: vec![event],
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                prepare_token_paste(ctx, token_id);
                ui.add(egui::TextEdit::singleline(&mut token).id(token_id));
                ui.add(egui::TextEdit::singleline(&mut other).id(other_id));
            });
        });
        (token, other)
    }

    #[test]
    fn token_paste_discards_only_trailing_clipboard_nul_terminators() {
        let (token, other) = edit_event(egui::Event::Paste("ghp_example\0".into()), true);
        assert_eq!(token, "ghp_example");
        assert!(other.is_empty());
        let (token, _) = edit_event(egui::Event::Paste("ghp_example".into()), true);
        assert_eq!(token, "ghp_example");
    }

    #[test]
    fn token_paste_preserves_embedded_and_non_nul_invalid_characters_for_validation() {
        for value in ["ghp_exam\0ple", "ghp_example\u{7f}", "ghp_example\u{200b}"] {
            let (token, _) = edit_event(egui::Event::Paste(value.into()), true);
            assert_eq!(token, value);
        }
    }

    #[test]
    fn token_paste_does_not_rewrite_other_fields_or_typed_input() {
        let value = "other-field\0";
        let (token, other) = edit_event(egui::Event::Paste(value.into()), false);
        assert!(token.is_empty());
        assert_eq!(other, value);
        let value = "typed-token\0";
        let (token, _) = edit_event(egui::Event::Text(value.into()), true);
        assert_eq!(token, value);
    }

    #[test]
    fn cancelling_replacement_keeps_saved_token_and_cancels_only_new_value() {
        let mut draft = TokenDraft {
            input: "replacement".into(),
            ..Default::default()
        };
        draft.stage();
        assert_eq!(draft.row_action_label(), "Undo");
        assert!(draft.cancel_pending_set());
        assert!(draft.change().is_none());
        assert_eq!(draft.row(true).unwrap().0, "Saved token");
        draft.remove();
        assert!(!draft.cancel_pending_set());
        assert!(matches!(draft.change(), Some(TokenChange::Remove)));
    }

    #[test]
    fn undo_pending_removal_keeps_the_saved_token() {
        let mut draft = TokenDraft::default();
        draft.remove();
        assert_eq!(draft.row_action_label(), "Undo");
        draft.undo_removal();
        assert!(draft.change().is_none());
        assert_eq!(draft.row(true).unwrap().0, "Saved token");
        assert_eq!(draft.row_action_label(), "Remove");
    }

    #[test]
    fn token_row_distinguishes_input_staged_changes_and_saved_credentials() {
        let mut draft = TokenDraft {
            input: "test-token".into(),
            ..Default::default()
        };
        assert!(
            draft.row(false).is_none(),
            "Pasting alone must not create a saved-token row"
        );
        assert_eq!(draft.row(true).unwrap().0, "Saved token");
        draft.stage();
        assert_eq!(draft.row(false).unwrap().0, "New token pending");
        assert_eq!(draft.row_action_label(), "Undo");
        draft.remove();
        assert_eq!(draft.row(true).unwrap().0, "Token removal pending");
        draft.discard();
        assert!(draft.row(false).is_none());
        assert_eq!(draft.row(true).unwrap().0, "Saved token");
    }

    #[test]
    fn staging_does_not_touch_credentials_and_cancel_discards_everything() {
        let mut draft = TokenDraft {
            input: "  test-token  ".into(),
            ..Default::default()
        };
        draft.stage();
        assert!(draft.input.is_empty());
        assert!(matches!(draft.change(), Some(TokenChange::Set(value)) if value == "test-token"));
        draft.discard();
        assert!(draft.change().is_none());
        draft.remove();
        assert!(matches!(draft.change(), Some(TokenChange::Remove)));
        draft.discard();
        assert!(draft.change().is_none());
    }

    #[test]
    fn save_includes_unstaged_input_and_latest_choice_wins() {
        let mut draft = TokenDraft {
            input: "replacement".into(),
            ..Default::default()
        };
        assert!(matches!(draft.change(), Some(TokenChange::Set(value)) if value == "replacement"));
        draft.remove();
        assert!(draft.input.is_empty());
        assert!(matches!(draft.change(), Some(TokenChange::Remove)));
        draft.input = "new-token".into();
        assert!(matches!(draft.change(), Some(TokenChange::Set(value)) if value == "new-token"));
        draft.stage();
        // Reading a change for a failed save must retain it for retry.
        assert!(matches!(draft.change(), Some(TokenChange::Set(_))));
        assert!(matches!(draft.change(), Some(TokenChange::Set(_))));
    }

    #[test]
    fn blank_input_never_replaces_a_saved_token() {
        let mut draft = TokenDraft {
            input: " \n ".into(),
            ..Default::default()
        };
        draft.stage();
        assert!(draft.change().is_none());
    }

    #[test]
    fn rejected_or_unverified_tokens_keep_the_explanation_visible() {
        use craft_apps_manager::network::TokenStatus;
        for status in [
            TokenStatus::Invalid,
            TokenStatus::UnableToVerify,
            TokenStatus::NotVerified,
        ] {
            assert!(!verification_allows_close(status));
        }
        assert!(verification_allows_close(TokenStatus::Connected));
        assert!(verification_allows_close(TokenStatus::NotConfigured));
    }
}
