// Design token system for OpenMango
// Inspired by VS Code Dark + MongoDB Green
// =============================================================================
// Colors
// =============================================================================

pub mod colors {
    use gpui::{Rgba, rgb, rgba};

    // -------------------------------------------------------------------------
    // Primitive Palette
    // -------------------------------------------------------------------------

    // Grays
    const GRAY_900: u32 = 0x1e1e1e; // Main Editor BG
    const GRAY_850: u32 = 0x252526; // Sidebar BG
    const GRAY_800: u32 = 0x2d2d2d; // Activity Bar / Headers
    const GRAY_600: u32 = 0x3e3e42; // Input BG
    const GRAY_500: u32 = 0x454545; // Borders
    const GRAY_400: u32 = 0x858585; // Muted Text
    const GRAY_300: u32 = 0xcccccc; // Secondary Text
    const GRAY_100: u32 = 0xffffff; // Primary Text

    // Accents
    const MONGO_GREEN: u32 = 0x00ED64;
    const MONGO_GREEN_DIM: u32 = 0x00C050;

    // Status
    const YELLOW_WARN: u32 = 0xCCA700;
    const GREEN_SUCCESS: u32 = 0x00ED64; // Same as brand
    const RED_DANGER: u32 = 0xDA3633;
    const RED_DANGER_HOVER: u32 = 0xF85149;
    const RED_ERROR_TEXT: u32 = 0xFCA5A5;

    // -------------------------------------------------------------------------
    // Semantic Tokens
    // -------------------------------------------------------------------------

    // Backgrounds
    pub fn bg_app() -> Rgba {
        rgb(GRAY_900)
    } // Main content area
    pub fn bg_sidebar() -> Rgba {
        rgb(GRAY_850)
    } // Sidebar / Panels
    pub fn bg_header() -> Rgba {
        rgb(GRAY_800)
    } // Section headers, status bar
    pub fn bg_hover() -> Rgba {
        rgb(0x2a2d2e)
    } // Subtle hover

    // Text
    pub fn text_primary() -> Rgba {
        rgb(GRAY_100)
    }
    pub fn text_secondary() -> Rgba {
        rgb(GRAY_300)
    }
    pub fn text_muted() -> Rgba {
        rgb(GRAY_400)
    }

    // Accents
    pub fn accent() -> Rgba {
        rgb(MONGO_GREEN)
    }
    pub fn accent_hover() -> Rgba {
        rgb(MONGO_GREEN_DIM)
    }

    // Borders
    pub fn border() -> Rgba {
        rgb(GRAY_500)
    }
    pub fn border_subtle() -> Rgba {
        rgb(0x333333)
    }
    pub fn border_focus() -> Rgba {
        accent()
    }

    // Status
    pub fn status_success() -> Rgba {
        rgb(GREEN_SUCCESS)
    }
    pub fn status_warning() -> Rgba {
        rgb(YELLOW_WARN)
    }
    pub fn status_error() -> Rgba {
        rgb(RED_DANGER)
    }
    pub fn text_error() -> Rgba {
        rgb(RED_ERROR_TEXT)
    }
    pub fn bg_error() -> Rgba {
        rgba(0xDA36331A)
    }
    pub fn border_error() -> Rgba {
        rgb(RED_DANGER)
    }

    // Components specific
    pub fn list_hover() -> Rgba {
        rgb(0x2a2d2e)
    }
    pub fn list_selected() -> Rgba {
        rgb(0x37373d)
    }
    pub fn bg_dirty() -> Rgba {
        rgba(0xF9C5131A)
    }

    // Buttons
    pub fn bg_button_primary() -> Rgba {
        accent()
    }
    pub fn bg_button_primary_hover() -> Rgba {
        accent_hover()
    }
    pub fn bg_button_secondary() -> Rgba {
        rgb(GRAY_600)
    }
    pub fn bg_button_secondary_hover() -> Rgba {
        rgb(0x4a4a4e)
    }
    pub fn text_button_primary() -> Rgba {
        rgb(0x000000)
    }
    pub fn bg_button_danger() -> Rgba {
        rgb(RED_DANGER)
    }
    pub fn bg_button_danger_hover() -> Rgba {
        rgb(RED_DANGER_HOVER)
    }
    pub fn text_button_danger() -> Rgba {
        rgb(0xFFFFFF)
    }

    // Syntax Highlighting (GitHub Dark)
    pub fn syntax_key() -> Rgba {
        rgb(0xC8E1FF)
    } // .pl-c1 meta.property-name
    pub fn syntax_string() -> Rgba {
        rgb(0x79B8FF)
    } // .pl-s string
    pub fn syntax_number() -> Rgba {
        rgb(0xFB8532)
    } // .pl-v variable
    pub fn syntax_boolean() -> Rgba {
        rgb(0x7BCC72)
    } // .pl-ent entity.name.tag
    pub fn syntax_null() -> Rgba {
        rgb(0x959DA5)
    } // .pl-c comment
    pub fn syntax_object_id() -> Rgba {
        rgb(0x56D4DD)
    } // cyan accent
    pub fn syntax_date() -> Rgba {
        rgb(0xB392F0)
    } // purple accent
    pub fn syntax_comment() -> Rgba {
        rgb(0x959DA5)
    }

    // -------------------------------------------------------------------------
    // Legacy mapping (for gradual migration if needed, but we will replace all)
    // -------------------------------------------------------------------------
    pub fn accent_green() -> Rgba {
        accent()
    }
}

// =============================================================================
// Spacing
// =============================================================================

pub mod spacing {
    use gpui::{Pixels, px};

    pub fn xs() -> Pixels {
        px(4.0)
    }
    pub fn sm() -> Pixels {
        px(8.0)
    }
    pub fn md() -> Pixels {
        px(12.0)
    }
    pub fn lg() -> Pixels {
        px(16.0)
    }
}

// =============================================================================
// Sizing
// =============================================================================

pub mod sizing {
    use gpui::{Pixels, px};

    // Layout
    pub fn sidebar_width() -> Pixels {
        px(260.0)
    } // Slightly wider
    pub fn status_bar_height() -> Pixels {
        px(22.0)
    } // VS Code style thin status bar
    pub fn header_height() -> Pixels {
        px(36.0)
    }

    // Elements
    pub fn icon_sm() -> Pixels {
        px(14.0)
    }
    pub fn icon_md() -> Pixels {
        px(16.0)
    } // Standard icon size
    pub fn icon_lg() -> Pixels {
        px(20.0)
    }

    pub fn button_height() -> Pixels {
        px(28.0)
    }

    pub fn status_dot() -> Pixels {
        px(8.0)
    }
}

// =============================================================================
// Typography
// =============================================================================

pub mod typography {
    use gpui::{Pixels, px};

    pub fn text_xs() -> Pixels {
        px(10.0)
    }
    pub fn text_sm() -> Pixels {
        px(12.0)
    } // Standard UI text
}

// =============================================================================
// Fonts
// =============================================================================

pub mod fonts {
    use gpui::relative;

    pub fn ui() -> &'static str {
        "JetBrainsMono Nerd Font"
    }
    pub fn heading() -> &'static str {
        "JetBrainsMono Nerd Font"
    }
    pub fn mono() -> &'static str {
        "Victor Mono"
    }
    pub fn ui_line_height() -> gpui::DefiniteLength {
        relative(1.45)
    }
}

// =============================================================================
// Borders
// =============================================================================

pub mod borders {
    use gpui::{Pixels, px};

    pub fn radius_sm() -> Pixels {
        px(3.0)
    } // Subtler rounded corners
}

// Re-exports for easier access
#[allow(unused_imports)]
pub use colors::*;
