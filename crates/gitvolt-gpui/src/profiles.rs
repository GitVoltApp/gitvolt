//! Profiles (`ui/src/tabs/ProfileSwitcher.tsx`, `ProfileDialog.tsx`).

use crate::app::GitVolt;
use crate::theme::{self, Col};
use gpui::*;

pub fn profile_color(this: &GitVolt) -> Hsla {
    this.profile["color"].as_str().and_then(Col::parse).map_or_else(theme::blue, Col::hsla)
}
