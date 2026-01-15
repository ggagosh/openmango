use crate::theme::{borders, colors, sizing, spacing, typography};
use gpui::*;

type ClickHandler = Box<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

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
}

impl RenderOnce for Button {
    fn render(self, _window: &mut Window, _cx: &mut App) -> impl IntoElement {
        let (bg, hover_bg, text_color, border_color) = match self.variant {
            ButtonVariant::Primary => (
                colors::bg_button_primary(),
                colors::bg_button_primary_hover(),
                colors::text_button_primary(),
                colors::bg_button_primary(), // No distinct border
            ),
            ButtonVariant::Danger => (
                colors::bg_button_danger(),
                colors::bg_button_danger_hover(),
                colors::text_button_danger(),
                colors::bg_button_danger(),
            ),
            ButtonVariant::Secondary => (
                colors::bg_button_secondary(),
                colors::bg_button_secondary_hover(),
                colors::text_primary(),
                colors::border_subtle(),
            ),
            ButtonVariant::Ghost => (
                gpui::rgba(0x00000000),
                colors::list_hover(),
                colors::text_primary(),
                gpui::rgba(0x00000000),
            ),
        };

        let height = if self.compact { px(22.0) } else { sizing::button_height() };

        let padding_x = if self.compact { spacing::sm() } else { spacing::md() };
        let padding_y = if self.compact { px(2.0) } else { px(4.0) };
        let text_size = if self.compact { typography::text_xs() } else { typography::text_sm() };
        // Keep buttons readable without feeling heavy in monospace UI fonts.
        let font_weight = FontWeight::NORMAL;

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

        if self.disabled {
            el = el.opacity(0.5).cursor_not_allowed();
        } else {
            el = el.cursor_pointer().hover(|s| s.bg(hover_bg));
            if let Some(handler) = self.on_click {
                el = el.on_click(handler);
            }
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
