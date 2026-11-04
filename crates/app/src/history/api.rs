//! What other verticals call to show commits.

use gpui_kit::base::TestSupportExt;
use gpui_kit::{
    Context, InteractiveElement, IntoElement, Render, StatefulInteractiveElement, Window,
};

use crate::ui;

/// The details of one or more commits: the log's side panel and the MR page's Commits tab.
pub struct CommitDetails {
    shas: Vec<String>,
}

impl CommitDetails {
    pub fn new(_repo: String, shas: Vec<String>, _cx: &mut Context<Self>) -> Self {
        CommitDetails { shas }
    }
}

impl Render for CommitDetails {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let short: Vec<_> = self.shas.iter().map(|s| &s[..s.len().min(10)]).collect();
        let short = short.join(", ");
        ui::placeholder(short.clone(), cx)
            .id("details")
            .test_support()
            .aria_label(short)
    }
}
