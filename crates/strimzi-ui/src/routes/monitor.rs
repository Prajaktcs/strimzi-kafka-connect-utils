use crate::views::{render, HtmlResult, MonitorPage};

pub async fn monitor() -> HtmlResult {
    render(MonitorPage { active: "monitor" })
}
