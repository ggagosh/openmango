use gpui_kit::*;

use super::Sidebar;

impl Sidebar {
    pub(super) fn handle_sidebar_key(
        &mut self,
        event: &KeyDownEvent,
        cx: &mut Context<Self>,
    ) -> bool {
        let key = event.keystroke.key.to_lowercase();
        if !self.model.search_open && self.handle_nav_key(&key, event.keystroke.modifiers, cx) {
            return true;
        }
        self.handle_typeahead_key(event, cx)
    }

    /// Tree navigation, shared by the focused tree and by focus resting on a control inside
    /// it. The arrows follow the usual tree contract: Right opens a closed folder and steps
    /// into an open one; Left closes an open folder and otherwise steps out to the parent.
    pub(super) fn handle_nav_key(
        &mut self,
        key: &str,
        modifiers: Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        match key {
            "up" | "arrowup" => self.move_sidebar_selection(-1, cx),
            "down" | "arrowdown" => self.move_sidebar_selection(1, cx),
            "home" => self.select_sidebar_first(cx),
            "end" => self.select_sidebar_last(cx),
            "pageup" => self.move_sidebar_page(-1, cx),
            "pagedown" => self.move_sidebar_page(1, cx),
            "left" | "arrowleft" => {
                let Some(node_id) = self.model.selected_tree_id.clone() else {
                    return true;
                };
                if self.model.expanded_nodes.contains(&node_id) {
                    // Alt closes everything below as well, as Option-click does in Finder.
                    self.set_node_expanded(&node_id, false, modifiers.alt, cx);
                } else if let Some(parent) = node_id.parent() {
                    self.select_sidebar_node(parent, true, cx);
                }
            }
            "right" | "arrowright" => {
                let Some(index) =
                    self.model.selected_tree_id.as_ref().and_then(|id| self.model.index_of(id))
                else {
                    return true;
                };
                if let Some(child) = self.model.first_child_index(index) {
                    let node_id = self.model.entries[child].id.clone();
                    self.select_sidebar_node(node_id, true, cx);
                } else if self.model.entries[index].is_folder {
                    // Opens a closed folder; on an open, empty one it retries the load.
                    let node_id = self.model.entries[index].id.clone();
                    self.set_node_expanded(&node_id, true, false, cx);
                }
            }
            _ => return false,
        }
        true
    }
}
