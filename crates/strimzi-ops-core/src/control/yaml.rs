use serde_json::{Map, Value};

/// Render a Strimzi `KafkaConnector` YAML document from a Connect config map.
pub fn to_strimzi_yaml(
    connector_name: &str,
    config: &Map<String, Value>,
    cluster_name: &str,
) -> String {
    let mut config = config.clone();
    let connector_class = config
        .remove("connector.class")
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    let tasks_max = config.remove("tasks.max").map_or_else(
        || "1".to_owned(),
        |value| match value {
            Value::Number(n) => n.to_string(),
            Value::String(s) => s,
            other => other.to_string(),
        },
    );

    let mut lines = vec![
        "apiVersion: kafka.strimzi.io/v1beta2".to_owned(),
        "kind: KafkaConnector".to_owned(),
        "metadata:".to_owned(),
        format!("  name: {connector_name}"),
        "  labels:".to_owned(),
        format!("    strimzi.io/cluster: {cluster_name}"),
        "spec:".to_owned(),
        format!("  class: {connector_class}"),
        format!("  tasksMax: {tasks_max}"),
        "  config:".to_owned(),
    ];

    let mut keys: Vec<_> = config.keys().cloned().collect();
    keys.sort();
    for key in keys {
        let Some(value) = config.get(&key) else {
            continue;
        };
        match value {
            Value::String(text) => {
                lines.push(format!(
                    "    {key}: \"{}\"",
                    escape_yaml_double_quoted(text)
                ));
            }
            other => lines.push(format!("    {key}: {other}")),
        }
    }

    lines.join("\n")
}

fn escape_yaml_double_quoted(text: &str) -> String {
    let mut escaped = String::with_capacity(text.len());
    for ch in text.chars() {
        match ch {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            other => escaped.push(other),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn exports_sorted_config_fields() {
        let mut config = Map::new();
        config.insert("connector.class".to_owned(), json!("io.demo.Source"));
        config.insert("tasks.max".to_owned(), json!("2"));
        config.insert("topic".to_owned(), json!("events"));
        config.insert("database.port".to_owned(), json!(5432));

        let yaml = to_strimzi_yaml("demo", &config, "my-connect-cluster");
        assert!(yaml.contains("name: demo"));
        assert!(yaml.contains("class: io.demo.Source"));
        assert!(yaml.contains("tasksMax: 2"));
        assert!(yaml.contains("    database.port: 5432"));
        assert!(yaml.contains("    topic: \"events\""));
        let topic_pos = yaml.find("topic:").expect("topic");
        let port_pos = yaml.find("database.port:").expect("port");
        assert!(port_pos < topic_pos);
    }

    #[test]
    fn escapes_special_characters_in_string_values() {
        let mut config = Map::new();
        config.insert(
            "query".to_owned(),
            json!("select \"id\" from t\nwhere a\\b"),
        );

        let yaml = to_strimzi_yaml("demo", &config, "cluster");
        assert!(yaml.contains("    query: \"select \\\"id\\\" from t\\nwhere a\\\\b\""));
    }
}
