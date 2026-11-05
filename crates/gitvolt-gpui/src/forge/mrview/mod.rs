//! The MR/PR view (`mrview/`).

pub mod checkout;

use crate::app::GitVolt;
use gpui::*;

impl GitVolt {
    /// `openMrView`: the tab's view of this MR/PR. Already open on it: left as it is.
    pub fn forge_open_mr(&mut self, tab: u32, number: u64, cx: &mut Context<Self>) {
        let Some(t) = self.tabs.iter_mut().find(|t| t.id == tab) else { return };
        if t.forge.open_mr == Some(number) {
            return;
        }
        t.forge.open_mr = Some(number);
        self.forge_refresh_mr(tab, number, cx);
        cx.notify();
    }
}
