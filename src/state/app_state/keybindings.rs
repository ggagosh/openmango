use anyhow::Result;

use super::AppState;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeybindingCapture {
    pub binding_id: String,
    pub shortcut: Option<String>,
}

impl AppState {
    pub fn keybinding_capture(&self) -> Option<&KeybindingCapture> {
        self.keybinding_capture.as_ref()
    }

    pub fn begin_keybinding_capture(&mut self, binding_id: String) {
        self.keybinding_capture = Some(KeybindingCapture { binding_id, shortcut: None });
    }

    pub fn capture_keybinding(&mut self, shortcut: String) {
        if let Some(capture) = self.keybinding_capture.as_mut() {
            capture.shortcut = Some(shortcut);
        }
    }

    pub fn cancel_keybinding_capture(&mut self) {
        self.keybinding_capture = None;
    }

    pub fn set_keybinding_override(
        &mut self,
        binding_id: String,
        shortcut: Option<String>,
    ) -> Result<()> {
        let mut next = self.settings.clone();
        next.keybindings.overrides.insert(binding_id, shortcut);
        self.config.save_settings(&next)?;
        self.settings = next;
        Ok(())
    }

    pub fn reset_keybinding(&mut self, binding_id: &str) -> Result<()> {
        let mut next = self.settings.clone();
        next.keybindings.overrides.remove(binding_id);
        self.config.save_settings(&next)?;
        self.settings = next;
        Ok(())
    }

    pub fn reset_all_keybindings(&mut self) -> Result<()> {
        let mut next = self.settings.clone();
        next.keybindings.overrides.clear();
        self.config.save_settings(&next)?;
        self.settings = next;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_tracks_candidate_and_cancels() {
        let mut state = AppState::new();

        state.begin_keybinding_capture("open-settings.workspace".into());
        state.capture_keybinding("cmd-alt-s".into());

        assert_eq!(
            state.keybinding_capture(),
            Some(&KeybindingCapture {
                binding_id: "open-settings.workspace".into(),
                shortcut: Some("cmd-alt-s".into()),
            })
        );
        state.cancel_keybinding_capture();
        assert!(state.keybinding_capture().is_none());
    }
}
