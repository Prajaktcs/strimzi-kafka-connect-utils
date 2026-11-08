use std::collections::BTreeMap;
use std::path::PathBuf;
use std::thread;
use std::time::{Duration, Instant};

use serde::Serialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::metrics::{ConnectorLag, MetricState, SinkLagCollector};
use crate::{ConnectClient, ConnectionSettings};

use super::process::{assertion, save, Result, Runner};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const BASELINE_ROWS: u64 = 2;
const BACKLOG_ROWS: u64 = 100;

/// The smoke intentionally targets the repository's local development stack.
#[derive(Debug, Clone)]
pub struct PipelineOptions {
    pub settings: ConnectionSettings,
    pub namespace: String,
    pub evidence_root: PathBuf,
    /// Each polling phase has its own bounded deadline.
    pub phase_timeout: Duration,
}

#[derive(Debug, Serialize)]
pub struct PipelineReport {
    pub run: String,
    pub namespace: String,
    pub source_connector: String,
    pub sink_connector: String,
    pub source_table: String,
    pub iceberg_table: String,
    pub topic: String,
    pub input_group: String,
    pub slot: String,
    pub publication: String,
    pub control_topic: String,
    pub evidence: PathBuf,
    pub verified_rows: u64,
}

struct Pipeline {
    options: PipelineOptions,
    report: PipelineReport,
    runner: Runner,
    client: ConnectClient,
    collector: SinkLagCollector,
}

/// Run a bounded, isolated `PostgreSQL` → Kafka → Iceberg proof. Resources are
/// deliberately retained on both success and failure; no offsets are ever reset.
pub fn run_pipeline(options: PipelineOptions) -> Result<PipelineReport> {
    if !(Duration::from_secs(30)..=Duration::from_secs(900)).contains(&options.phase_timeout) {
        return Err(assertion(
            "phase timeout must be between 30 and 900 seconds",
        ));
    }
    let run = format!("pipeline-smoke-{}", Uuid::new_v4().simple());
    let sql_id = run.replace('-', "_");
    let evidence = options.evidence_root.join(&run);
    let report = PipelineReport {
        source_connector: format!("{run}-source"),
        sink_connector: format!("{run}-sink"),
        source_table: format!("public.{sql_id}"),
        iceberg_table: format!("public.{sql_id}"),
        topic: format!("{run}.public.{sql_id}"),
        input_group: format!("{run}-input"),
        slot: sql_id.clone(),
        publication: format!("{sql_id}_pub"),
        control_topic: format!("{run}-control"),
        namespace: options.namespace.clone(),
        evidence: evidence.clone(),
        run,
        verified_rows: 0,
    };
    let runner = Runner::new(evidence)?;
    save_json(&runner, "resources.json", &report)?;
    let connect_url = options
        .settings
        .connect_url
        .as_deref()
        .unwrap_or("http://127.0.0.1:8083");
    let client = ConnectClient::with_timeout(connect_url, Duration::from_secs(5))
        .map_err(|_| assertion("cannot initialize Connect client; check --connect-url"))?;
    let collector = SinkLagCollector::new(
        Some(connect_url.to_owned()),
        Some(
            options
                .settings
                .bootstrap_servers
                .clone()
                .unwrap_or_else(|| "127.0.0.1:9092".to_owned()),
        ),
    )
    .with_security(options.settings.sink_lag_security.clone());
    let mut pipeline = Pipeline {
        options,
        report,
        runner,
        client,
        collector,
    };
    match pipeline.execute() {
        Ok(()) => {
            pipeline.report.verified_rows = BASELINE_ROWS + BACKLOG_ROWS;
            save_json(&pipeline.runner, "success.json", &pipeline.report)?;
            Ok(pipeline.report)
        }
        Err(error) => {
            let _ = save(
                &pipeline.runner.directory.join("failure.txt"),
                error.to_string().as_bytes(),
            );
            Err(assertion(format!(
                "{error}; resources retained; evidence: {}",
                pipeline.runner.directory.display()
            )))
        }
    }
}

fn save_json(runner: &Runner, name: &str, value: &impl Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| assertion("cannot serialize smoke evidence"))?;
    save(&runner.directory.join(name), &bytes)
}

impl Pipeline {
    fn kubectl(&mut self, label: &str, args: &[&str], input: Option<&str>) -> Result<String> {
        let mut full = vec![
            "--request-timeout=20s",
            "-n",
            self.options.namespace.as_str(),
        ];
        full.extend_from_slice(args);
        self.runner
            .command(label, "kubectl", &full, input, COMMAND_TIMEOUT)
    }

    fn postgres(&mut self, label: &str, sql: &str) -> Result<String> {
        self.kubectl(
            label,
            &[
                "exec",
                "-i",
                "postgres-0",
                "--",
                "psql",
                "-X",
                "-U",
                "postgres",
                "-d",
                "source_db",
                "-v",
                "ON_ERROR_STOP=1",
                "-At",
            ],
            Some(sql),
        )
    }

    fn kafka(&mut self, label: &str, tool: &str, args: &[&str]) -> Result<String> {
        let tool = format!("/opt/kafka/bin/{tool}.sh");
        let mut full = vec![
            "exec",
            "my-cluster-dual-role-0",
            "--",
            tool.as_str(),
            "--bootstrap-server",
            "my-cluster-kafka-bootstrap:9092",
        ];
        full.extend_from_slice(args);
        self.kubectl(label, &full, None)
    }

    fn duckdb(&mut self, label: &str, query: &str) -> Result<String> {
        let sql = format!(
            "INSTALL iceberg; LOAD iceberg; INSTALL httpfs; LOAD httpfs;\n\
             CREATE SECRET smoke_s3 (TYPE s3, KEY_ID 'rustfsadmin', SECRET 'rustfsadmin',\
             REGION 'us-east-1', ENDPOINT '127.0.0.1:9000', URL_STYLE 'path', USE_SSL false);\n\
             ATTACH 'warehouse' AS rustfs (TYPE iceberg, ENDPOINT 'http://127.0.0.1:9000/iceberg',\
             AUTHORIZATION_TYPE 'sigv4', SECRET 'smoke_s3', SIGV4_REGION 'us-east-1',\
             SIGV4_SERVICE 's3', ACCESS_DELEGATION_MODE 'none');\n{query}\n"
        );
        self.runner.command(
            label,
            "duckdb",
            &["-batch", "-bail", "-init", "/dev/null", "-json", ":memory:"],
            Some(&sql),
            COMMAND_TIMEOUT,
        )
    }

    fn template(&mut self, name: &str, expected_class: &str) -> Result<Value> {
        let text = self.kubectl(
            "connector-template",
            &["get", "kafkaconnector", name, "-o", "json"],
            None,
        )?;
        let value: Value = serde_json::from_str(&text)
            .map_err(|_| assertion("cannot decode sample KafkaConnector"))?;
        if value.pointer("/spec/class").and_then(Value::as_str) != Some(expected_class)
            || value
                .pointer("/spec/config")
                .and_then(Value::as_object)
                .is_none()
            || value
                .pointer("/metadata/labels/strimzi.io~1cluster")
                .and_then(Value::as_str)
                != Some(self.options.settings.cluster_name())
        {
            return Err(assertion(format!(
                "sample {name} class/config/cluster differs from the local stack; restore sample manifests"
            )));
        }
        Ok(value)
    }

    fn execute(&mut self) -> Result<()> {
        // Resolve all external prerequisites before creating any cluster resources.
        self.kubectl("cluster-prerequisite", &["get", "pods", "-o", "name"], None)?;
        self.client.get_cluster_info().map_err(|_| {
            assertion("cannot reach Connect; start local stack and run `just port-forward-all`")
        })?;
        let source = self.template(
            "postgres-source-connector",
            "io.debezium.connector.postgresql.PostgresConnector",
        )?;
        let sink = self.template(
            "iceberg-sink-connector",
            "org.apache.iceberg.connect.IcebergSinkConnector",
        )?;
        require_config(&source, "database.hostname", "postgres")?;
        require_config(&source, "database.dbname", "source_db")?;
        require_config(&sink, "iceberg.catalog.uri", "http://rustfs:9000/iceberg")?;
        self.postgres("postgres-prerequisite", "SELECT 1;")?;
        self.duckdb("duckdb-prerequisite", "SELECT count(*) AS namespaces FROM information_schema.schemata WHERE catalog_name='rustfs';")?;
        let cluster = self.kubectl(
            "kafka-prerequisite",
            &[
                "exec",
                "my-cluster-dual-role-0",
                "--",
                "/opt/kafka/bin/kafka-cluster.sh",
                "cluster-id",
                "--bootstrap-server",
                "my-cluster-kafka-bootstrap:9092",
            ],
            None,
        )?;
        let expected = self
            .client
            .get_cluster_info()
            .map_err(|_| assertion("cannot read Connect cluster identity"))?
            .kafka_cluster_id
            .ok_or_else(|| assertion("Connect Kafka cluster ID is unavailable"))?;
        if !cluster.split_whitespace().any(|word| word == expected) {
            return Err(assertion("Connect and in-cluster Kafka cluster IDs differ"));
        }
        // Exercise host metadata/security/cluster validation before mutations too.
        let prerequisite = self.collector.collect();
        if prerequisite.error.is_some()
            || prerequisite.connectors.is_empty()
            || prerequisite.connectors.iter().any(|row| {
                row.state == MetricState::CollectionError || row.state == MetricState::Stale
            })
        {
            return Err(assertion("host sink-lag collection failed; check Kafka forward, advertised listener and sink-lag security settings"));
        }
        let source = self.clone_source(source)?;
        let sink = self.clone_sink(sink)?;
        save_json(&self.runner, "source.json", &source)?;
        save_json(&self.runner, "sink.json", &sink)?;
        let table = self.report.source_table.clone();
        self.postgres("create-source-table", &format!(
            "SET statement_timeout='20s'; CREATE TABLE {table} (id bigint PRIMARY KEY, name text NOT NULL, email text NOT NULL);"
        ))?;
        let topic = self.report.topic.clone();
        self.kafka(
            "create-topic",
            "kafka-topics",
            &[
                "--create",
                "--topic",
                &topic,
                "--partitions",
                "2",
                "--replication-factor",
                "1",
            ],
        )?;
        self.create_connector(&source)?;
        self.create_connector(&sink)?;
        self.wait_state(true, "RUNNING")?;
        self.wait_state(false, "RUNNING")?;
        self.wait_source_slot()?;
        self.insert_rows(1, BASELINE_ROWS, "seed")?;
        let baseline = self.wait_lag(0, None, "baseline")?;
        // Commit progress alone is not a durability proof: query baseline files too.
        self.wait_rows(BASELINE_ROWS, "baseline-rows")?;
        self.state("paused")?;
        self.wait_state(false, "PAUSED")?;
        // Allow outstanding commit/rebalance callbacks to settle before observation.
        thread::sleep(Duration::from_secs(3));
        self.insert_rows(BASELINE_ROWS + 1, BASELINE_ROWS + BACKLOG_ROWS, "backlog")?;
        let backlog = self.wait_lag(BACKLOG_ROWS / 2, Some(&baseline), "backlog")?;
        self.observe_paused(&backlog)?;
        self.state("running")?;
        self.wait_state(false, "RUNNING")?;
        self.wait_lag(0, Some(&backlog), "recovered")?;
        self.wait_rows(BASELINE_ROWS + BACKLOG_ROWS, "durable-rows")?;
        Ok(())
    }

    fn clone_source(&self, mut value: Value) -> Result<Value> {
        self.isolate(&mut value, &self.report.source_connector);
        let config = value
            .pointer_mut("/spec/config")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| assertion("source config is missing"))?;
        // No inherited transforms, routing, notifications or signal channels may
        // write into the sample's topics or include its tables.
        config.retain(|key, _| {
            !key.starts_with("transforms")
                && !key.starts_with("signal.")
                && !key.starts_with("notification.")
                && !key.starts_with("topic.creation.")
                && !key.starts_with("heartbeat.")
                && !matches!(
                    key.as_str(),
                    "table.exclude.list"
                        | "schema.exclude.list"
                        | "column.include.list"
                        | "column.exclude.list"
                        | "snapshot.select.statement.overrides"
                )
        });
        let table_pattern = self.report.source_table.replace('.', "\\.");
        let history_topic = format!("{}-history", self.report.run);
        for (key, value) in [
            ("topic.prefix", self.report.run.as_str()),
            ("table.include.list", table_pattern.as_str()),
            ("schema.include.list", "public"),
            ("slot.name", self.report.slot.as_str()),
            ("publication.name", self.report.publication.as_str()),
            ("publication.autocreate.mode", "filtered"),
            ("snapshot.mode", "no_data"),
            ("heartbeat.interval.ms", "0"),
            (
                "schema.history.internal.kafka.topic",
                history_topic.as_str(),
            ),
            (
                "producer.override.partitioner.class",
                "org.apache.kafka.clients.producer.RoundRobinPartitioner",
            ),
            ("tombstones.on.delete", "false"),
            ("slot.drop.on.stop", "false"),
        ] {
            config.insert(key.to_owned(), json!(value));
        }
        config.insert(
            "custom.metric.tags".to_owned(),
            json!(format!("connector={},task=0", self.report.source_connector)),
        );
        Ok(value)
    }

    fn clone_sink(&self, mut value: Value) -> Result<Value> {
        self.isolate(&mut value, &self.report.sink_connector);
        let config = value
            .pointer_mut("/spec/config")
            .and_then(Value::as_object_mut)
            .ok_or_else(|| assertion("sink config is missing"))?;
        config.retain(|key, _| {
            !key.starts_with("transforms")
                && key != "topics.regex"
                && !key.starts_with("iceberg.table.")
                && !key.starts_with("errors.deadletterqueue.")
                && !matches!(
                    key.as_str(),
                    "iceberg.tables.route-field" | "iceberg.tables.dynamic-enabled"
                )
        });
        for (key, value) in [
            ("topics", self.report.topic.clone()),
            (
                "consumer.override.group.id",
                self.report.input_group.clone(),
            ),
            ("consumer.override.auto.offset.reset", "earliest".to_owned()),
            ("iceberg.connect.group-id", self.report.input_group.clone()),
            ("iceberg.tables", self.report.iceberg_table.clone()),
            ("iceberg.control.topic", self.report.control_topic.clone()),
            (
                "iceberg.control.group-id-prefix",
                format!("{}-control-group", self.report.run),
            ),
            (
                "iceberg.coordinator.transactional.prefix",
                format!("{}-", self.report.run),
            ),
            ("iceberg.control.commit.interval-ms", "10000".to_owned()),
        ] {
            config.insert(key.to_owned(), json!(value));
        }
        Ok(value)
    }

    fn isolate(&self, value: &mut Value, name: &str) {
        value["metadata"] = json!({"name": name, "namespace": self.options.namespace,
            "labels": {"strimzi.io/cluster": self.options.settings.cluster_name(),
                       "strimzi-ops/smoke": self.report.run}});
        if let Some(object) = value.as_object_mut() {
            object.remove("status");
        }
        value["spec"]["tasksMax"] = json!(1);
        value["spec"]["state"] = json!("running");
    }

    fn create_connector(&mut self, value: &Value) -> Result<()> {
        let input =
            serde_json::to_string(value).map_err(|_| assertion("cannot encode smoke connector"))?;
        self.kubectl("create-connector", &["create", "-f", "-"], Some(&input))?;
        Ok(())
    }

    fn state(&mut self, state: &str) -> Result<()> {
        let name = self.report.sink_connector.clone();
        let patch = json!({"spec": {"state": state}}).to_string();
        self.kubectl(
            "desired-state",
            &[
                "patch",
                "kafkaconnector",
                &name,
                "--type=merge",
                "-p",
                &patch,
            ],
            None,
        )?;
        Ok(())
    }

    fn wait_state(&mut self, source: bool, expected: &str) -> Result<()> {
        let name = if source {
            &self.report.source_connector
        } else {
            &self.report.sink_connector
        };
        let deadline = Instant::now() + self.options.phase_timeout;
        loop {
            if let Ok(status) = self.client.get_connector_status(name) {
                if status.pointer("/connector/state").and_then(Value::as_str) == Some(expected)
                    && status
                        .get("tasks")
                        .and_then(Value::as_array)
                        .is_some_and(|tasks| {
                            !tasks.is_empty()
                                && tasks.iter().all(|task| {
                                    task.get("state").and_then(Value::as_str) == Some(expected)
                                })
                        })
                {
                    return Ok(());
                }
                if status.pointer("/connector/state").and_then(Value::as_str) == Some("FAILED")
                    || status
                        .get("tasks")
                        .and_then(Value::as_array)
                        .is_some_and(|tasks| {
                            tasks.iter().any(|task| {
                                task.get("state").and_then(Value::as_str) == Some("FAILED")
                            })
                        })
                {
                    // Do not persist REST traces, which may include credentials.
                    return Err(assertion(format!(
                        "connector {name} failed; inspect its task logs"
                    )));
                }
            }
            if Instant::now() >= deadline {
                return Err(assertion(format!(
                    "connector {name} did not reach {expected}"
                )));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn wait_source_slot(&mut self) -> Result<()> {
        let deadline = Instant::now() + self.options.phase_timeout;
        let query = format!(
            "SELECT active FROM pg_replication_slots WHERE slot_name='{}' AND database='source_db';",
            self.report.slot
        );
        loop {
            // RUNNING is reported before Debezium necessarily starts replication.
            // With no_data snapshots, inserts before slot creation can be missed.
            if self.postgres("source-slot-ready", &query)?.trim() == "t" {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(assertion(
                    "isolated source replication slot did not become active",
                ));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn insert_rows(&mut self, first: u64, last: u64, phase: &str) -> Result<()> {
        let sql = format!(
            "SET statement_timeout='20s'; INSERT INTO {} SELECT id, 'smoke-' || id, 'smoke-' || id || '@example.test' FROM generate_series({first}, {last}) AS id;",
            self.report.source_table
        );
        self.postgres(phase, &sql)?;
        Ok(())
    }

    fn wait_lag(
        &mut self,
        each: u64,
        prior: Option<&BTreeMap<i32, (i64, i64)>>,
        label: &str,
    ) -> Result<BTreeMap<i32, (i64, i64)>> {
        let deadline = Instant::now() + self.options.phase_timeout;
        let mut attempt = 0_u32;
        loop {
            let snapshot = self.collector.collect();
            attempt += 1;
            let measured = snapshot
                .connectors
                .iter()
                .find(|row| row.connector == self.report.sink_connector);
            save_json(
                &self.runner,
                &format!("{label}-sample-{attempt}.json"),
                &json!({
                    "state": format!("{:?}", snapshot.state), "error": snapshot.error,
                    "connector": measured.map(|row| json!({
                        "state": format!("{:?}", row.state), "error": row.error,
                        "group": row.group, "total_lag": row.total_lag,
                        "partitions": row.partitions.iter().map(|part| json!({
                            "topic": part.topic, "partition": part.partition,
                            "committed": part.committed, "end": part.end, "lag": part.lag,
                            "state": format!("{:?}", part.state), "error": part.error,
                        })).collect::<Vec<_>>(),
                    })),
                }),
            )?;
            if let Some(row) = snapshot
                .connectors
                .iter()
                .find(|row| row.connector == self.report.sink_connector)
            {
                if let Some(offsets) = exact_lag(
                    row,
                    &self.report.topic,
                    &self.report.input_group,
                    each,
                    prior,
                ) {
                    save_json(&self.runner, &format!("{label}-offsets.json"), &offsets)?;
                    return Ok(offsets);
                }
            }
            if Instant::now() >= deadline {
                return Err(assertion(format!("{label}: expected fresh complete lag {} ({} per partition), not stale/missing offsets; inspect retained connectors", each * 2, each)));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    fn group_description(&mut self, mode: &str) -> Result<String> {
        let group = self.report.input_group.clone();
        let mut args = vec!["--group", &group, "--describe", mode];
        if mode == "--members" {
            args.push("--verbose");
        }
        self.kafka("paused-group-observation", "kafka-consumer-groups", &args)
    }

    fn observe_paused(&mut self, backlog: &BTreeMap<i32, (i64, i64)>) -> Result<()> {
        let before_offsets = canonical_description(&self.group_description("--offsets")?);
        let before_members = canonical_description(&self.group_description("--members")?);
        // Two real read-only collector rounds inside a stable paused window.
        for _ in 0..2 {
            thread::sleep(Duration::from_secs(10));
            let snapshot = self.collector.collect();
            let row = snapshot
                .connectors
                .iter()
                .find(|row| row.connector == self.report.sink_connector)
                .ok_or_else(|| assertion("paused sink disappeared from collector"))?;
            let measured = exact_lag(
                row,
                &self.report.topic,
                &self.report.input_group,
                BACKLOG_ROWS / 2,
                None,
            )
            .ok_or_else(|| assertion("paused backlog is not fresh, exact and complete"))?;
            if &measured != backlog {
                return Err(assertion("observer window offsets changed"));
            }
        }
        let after_offsets = canonical_description(&self.group_description("--offsets")?);
        let after_members = canonical_description(&self.group_description("--members")?);
        if before_offsets != after_offsets || before_members != after_members {
            return Err(assertion("paused input group members/assignments/offsets changed during read-only observation"));
        }
        save(
            &self.runner.directory.join("observer-invariance.txt"),
            b"Input group offset and member descriptions unchanged across two collector rounds.\n",
        )?;
        Ok(())
    }

    fn wait_rows(&mut self, count: u64, label: &str) -> Result<()> {
        let deadline = Instant::now() + self.options.phase_timeout;
        let query = format!(
            "SELECT \"after\".id AS id, \"after\".name AS name, \"after\".email AS email, op FROM rustfs.{} ORDER BY id;",
            self.report.iceberg_table
        );
        loop {
            if let Ok(text) = self.duckdb(label, &query) {
                if let Some(rows) = final_json_result(&text) {
                    if exact_rows(&rows, count) {
                        save_json(&self.runner, &format!("{label}-verified.json"), &rows)?;
                        return Ok(());
                    }
                }
            }
            if Instant::now() >= deadline {
                return Err(assertion(format!("DuckDB did not independently read exactly {count} expected CDC rows (id/name/email/op=c) before deadline")));
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

fn require_config(template: &Value, key: &str, expected: &str) -> Result<()> {
    if template["spec"]["config"][key].as_str() != Some(expected) {
        return Err(assertion(format!(
            "sample setting {key} differs from supported local-stack defaults"
        )));
    }
    Ok(())
}

fn canonical_description(text: &str) -> Vec<Vec<String>> {
    let mut lines: Vec<_> = text
        .lines()
        .map(|line| {
            line.split_whitespace()
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .filter(|line| !line.is_empty())
        .collect();
    lines.sort_unstable();
    lines
}

fn exact_lag(
    row: &ConnectorLag,
    topic: &str,
    group: &str,
    each: u64,
    prior: Option<&BTreeMap<i32, (i64, i64)>>,
) -> Option<BTreeMap<i32, (i64, i64)>> {
    if row.state != MetricState::Available
        || row.group.as_deref() != Some(group)
        || row.total_lag != Some(each * 2)
        || row.partitions.len() != 2
    {
        return None;
    }
    let mut offsets = BTreeMap::new();
    for partition in &row.partitions {
        if partition.state != MetricState::Available
            || partition.topic != topic
            || !(0..2).contains(&partition.partition)
            || partition.lag != Some(each)
        {
            return None;
        }
        let (committed, end) = (partition.committed?, partition.end?);
        if committed < 1 || end.checked_sub(committed)? != i64::try_from(each).ok()? {
            return None;
        }
        if let Some(prior) = prior {
            let (old_commit, old_end) = prior.get(&partition.partition)?;
            if each > 0
                && (committed != *old_commit || end - old_end != i64::try_from(each).ok()?)
            {
                return None;
            }
            if each == 0 && (end != *old_end || committed <= *old_commit) {
                return None;
            }
        }
        if offsets
            .insert(partition.partition, (committed, end))
            .is_some()
        {
            return None;
        }
    }
    Some(offsets)
}

fn final_json_result(text: &str) -> Option<Value> {
    // DuckDB emits one JSON value per statement, including CREATE SECRET's
    // success row. Only the final SELECT result contains the durable row proof.
    serde_json::Deserializer::from_str(text)
        .into_iter::<Value>()
        .last()?
        .ok()
}

fn exact_rows(value: &Value, count: u64) -> bool {
    let Some(rows) = value.as_array() else {
        return false;
    };
    u64::try_from(rows.len()).ok() == Some(count)
        && rows.iter().zip(1..=count).all(|(row, id)| {
            row.get("id").and_then(Value::as_u64) == Some(id)
                && row.get("name").and_then(Value::as_str) == Some(format!("smoke-{id}").as_str())
                && row.get("email").and_then(Value::as_str)
                    == Some(format!("smoke-{id}@example.test").as_str())
                && row.get("op").and_then(Value::as_str) == Some("c")
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::PartitionLag;

    #[test]
    fn reads_final_duckdb_result_without_accepting_truncated_or_intermediate_rows() {
        let output = concat!(
            "[{\"Success\":true}]\n",
            "[{\"id\":1,\"name\":\"smoke-1\",\"email\":\"smoke-1@example.test\",\"op\":\"c\"}]\n"
        );
        assert!(exact_rows(&final_json_result(output).unwrap(), 1));
        assert!(final_json_result(&format!("{output}[{{")).is_none());
        assert!(final_json_result(&format!("[{{\"Success\":tru}}]\n{output}")).is_none());
        assert!(!exact_rows(
            &final_json_result(&format!("{output}[{{\"Success\":true}}]")).unwrap(),
            1
        ));
    }

    #[test]
    fn row_proof_rejects_duplicates_missing_values_and_snapshot_events() {
        let rows = json!([
            {"id":1,"name":"smoke-1","email":"smoke-1@example.test","op":"c"},
            {"id":2,"name":"smoke-2","email":"smoke-2@example.test","op":"c"}
        ]);
        assert!(exact_rows(&rows, 2));
        let mut duplicate = rows.clone();
        duplicate[1] = duplicate[0].clone();
        assert!(!exact_rows(&duplicate, 2));
        let mut snapshot = rows.clone();
        snapshot[0]["op"] = json!("r");
        assert!(!exact_rows(&snapshot, 2));
        let mut missing = rows;
        missing[1]["email"] = Value::Null;
        assert!(!exact_rows(&missing, 2));
    }

    #[test]
    fn lag_proof_requires_fresh_complete_partitions_and_unchanged_paused_commits() {
        let mut row = ConnectorLag {
            connector: "sink".to_owned(),
            group: Some("group".to_owned()),
            state: MetricState::Available,
            total_lag: Some(100),
            error: None,
            partitions: (0..2)
                .map(|partition| PartitionLag {
                    topic: "topic".to_owned(),
                    partition,
                    committed: Some(1),
                    end: Some(51),
                    lag: Some(50),
                    state: MetricState::Available,
                    error: None,
                })
                .collect(),
        };
        let baseline = BTreeMap::from([(0, (1, 1)), (1, (1, 1))]);
        assert!(exact_lag(&row, "topic", "group", 50, Some(&baseline)).is_some());
        row.partitions[1].state = MetricState::Stale;
        assert!(exact_lag(&row, "topic", "group", 50, Some(&baseline)).is_none());
        row.partitions[1].state = MetricState::Available;
        row.partitions[1].committed = Some(2);
        assert!(exact_lag(&row, "topic", "group", 50, Some(&baseline)).is_none());
        row.partitions[1] = row.partitions[0].clone();
        assert!(exact_lag(&row, "topic", "group", 50, None).is_none());
    }
}
