//! Cluster-scoped UI paths.

pub fn dashboard(cluster: &str) -> String {
    format!("/c/{cluster}/dashboard")
}

pub fn monitor(cluster: &str) -> String {
    format!("/c/{cluster}/monitor")
}

pub fn control(cluster: &str) -> String {
    format!("/c/{cluster}/control")
}

pub fn control_focus(cluster: &str, name: &str, flash: Option<&str>) -> String {
    match flash {
        Some(msg) => format!("/c/{cluster}/control?flash={msg}&focus={name}"),
        None => format!("/c/{cluster}/control?focus={name}"),
    }
}
