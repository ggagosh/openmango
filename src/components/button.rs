use crate::theme::{borders, sizing, spacing, typography};
use gpui::*;
use gpui_component::{ActiveTheme as _, tooltip::Tooltip};
use std::rc::Rc;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;
type TooltipAction = (Rc<Box<dyn Action>>, Option<SharedString>);

#[derive(Clone, Copy, PartialEq, Default)]
pub enum ButtonVariant {
    #[default]
    Secondary,
    Primary,
    Danger,
    Ghost,
}

#[derive(IntoElement)]
pub struct Button {
    id: ElementId,
    label: Option<SharedString>,
    icon: Option<AnyElement>,
    icon_right: bool,
    variant: ButtonVariant,
    on_click: Option<ClickHandler>,
    disabled: bool,
    compact: bool,
    focus_handle: Option<FocusHandle>,
    tab_index: Option<isize>,
    tooltip: Option<(SharedString, Option<TooltipAction>)>,
}

impl Button {
    pub fn new(id: impl Into<ElementId>) -> Self {
        Self {
            id: id.into(),
            label: None,
            icon: None,
            icon_right: false,
            variant: ButtonVariant::Secondary,
            on_click: None,
            disabled: false,
            compact: false,
            focus_handle: None,
            tab_index: None,
            tooltip: None,
        }
    }

    pub fn primary(mut self) -> Self {
        self.variant = ButtonVariant::Primary;
        self
    }

    pub fn ghost(mut self) -> Self {
        self.variant = ButtonVariant::Ghost;
        self
    }

    pub fn danger(mut self) -> Self {
        self.variant = ButtonVariant::Danger;
        self
    }

    pub fn compact(mut self) -> Self {
        self.compact = true;
        self
    }

    pub fn label(mut self, label: impl Into<SharedString>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn icon(mut self, icon: impl IntoElement) -> Self {
        self.icon = Some(icon.into_any_element());
        self
    }

    pub fn icon_right(mut self) -> Self {
        self.icon_right = true;
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_click = Some(Box::new(handler));
        self
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn track_focus(mut self, focus_handle: &FocusHandle) -> Self {
        self.focus_handle = Some(focus_handle.clone());
        self
    }

    pub fn tab_index(mut self, index: isize) -> Self {
        self.tab_index = Some(index);
        self
    }

    pub fn tooltip(mut self, tooltip: impl Into<SharedString>) -> Self {
        self.tooltip = Some((tooltip.into(), None));
        self
    }

    pub fn tooltip_with_action(
        mut self,
        tooltip: impl Into<SharedString>,
        action: &dyn Action,
        context: Option<&str>,
    ) -> Self {
        self.tooltip = Some((
            tooltip.into(),
            Some((action.boxed_clone().into(), context.map(SharedString::new))),
        ));
        self
    }
}

impl RenderOnce for Button {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let (bg, hover_bg, text_color, border_color) = match self.variant {
            ButtonVariant::Primary => (
                cx.theme().primary,
                cx.theme().primary_hover,
                cx.theme().primary_foreground,
                cx.theme().primary, // No distinct border
            ),
            ButtonVariant::Danger => (
                cx.theme().danger,
                cx.theme().danger_hover,
                cx.theme().danger_foreground,
                cx.theme().danger,
            ),
            ButtonVariant::Secondary => (
                cx.theme().secondary,
                cx.theme().secondary_hover,
                cx.theme().foreground,
                cx.theme().sidebar_border,
            ),
            ButtonVariant::Ghost => (
                crate::theme::colors::transparent(),
                cx.theme().list_hover,
                cx.theme().foreground,
                crate::theme::colors::transparent(),
            ),
        };

        let height = if self.compact { px(22.0) } else { sizing::button_height() };

        let padding_x = if self.compact { spacing::sm() } else { spacing::md() };
        let padding_y = if self.compact { px(2.0) } else { px(4.0) };
        let text_size = if self.compact { typography::text_xs() } else { typography::text_sm() };
        // Keep buttons readable without feeling heavy in monospace UI fonts.
        let font_weight = FontWeight::NORMAL;
        let focus_handle = self.focus_handle.clone();
        let tab_index = self.tab_index;
        let tooltip = self.tooltip.clone();
        let is_focused =
            focus_handle.as_ref().is_some_and(|focus_handle| focus_handle.is_focused(window));

        let mut el = div()
            .id(self.id)
            .flex()
            .items_center()
            .justify_center()
            .h(height)
            .px(padding_x)
            .py(padding_y)
            .rounded(borders::radius_sm())
            .border_1()
            .border_color(border_color)
            .bg(bg)
            .text_size(text_size)
            .font_weight(font_weight)
            .text_color(text_color);

        if let Some(focus_handle) = focus_handle {
            let mut focus_handle = focus_handle;
            if let Some(tab_index) = tab_index {
                focus_handle = focus_handle.tab_index(tab_index);
                focus_handle = focus_handle.tab_stop(true);
            }
            el = el.track_focus(&focus_handle);
        }

        if let Some(tab_index) = tab_index {
            el = el.tab_index(tab_index);
        }

        if is_focused && !self.disabled {
            el = el.border_color(cx.theme().ring).shadow_xs();
        }

        if self.disabled {
            el = el.opacity(0.5).cursor_not_allowed();
        } else {
            el = el.cursor_pointer().hover(|s| s.bg(hover_bg));
            if let Some(handler) = self.on_click {
                el = el.on_click(handler);
            }
        }

        if let Some((tooltip, action)) = tooltip {
            el = el.tooltip(move |window, cx| {
                let mut tooltip_el = Tooltip::new(tooltip.clone());
                if let Some((action, context)) = action.clone() {
                    tooltip_el = tooltip_el.action(
                        action.boxed_clone().as_ref(),
                        context.as_ref().map(|c| c.as_ref()),
                    );
                }
                tooltip_el.build(window, cx)
            });
        }

        let label = self.label;
        let icon = self.icon;
        let has_label = label.is_some();

        if self.icon_right {
            if let Some(label) = label {
                el = el.child(label);
            }
            if let Some(icon) = icon {
                if has_label {
                    el = el.gap(spacing::xs());
                }
                el = el.child(icon);
            }
        } else {
            if let Some(icon) = icon {
                el = el.child(icon);
                if has_label {
                    el = el.gap(spacing::xs());
                }
            }

            if let Some(label) = label {
                el = el.child(label);
            }
        }

        el
    }
}
