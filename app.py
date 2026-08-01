"""
Strimzi Ops Platform - Streamlit Application

A unified platform to validate, monitor, and control Kafka Connect deployments.
"""

import json

import streamlit as st

# Page configuration
st.set_page_config(page_title="Strimzi Ops Platform", page_icon="🔌", layout="wide")

# Initialize session state
if "config" not in st.session_state:
    try:
        from strimzi_ops.config import Config
        from strimzi_ops.control import ConnectorController

        st.session_state.config = Config()
        st.session_state.controller = ConnectorController(
            st.session_state.config.kafka_connect_url,
            st.session_state.config.kafka_bootstrap_servers,
        )
    except FileNotFoundError:
        # Config file doesn't exist - set to None
        # Linter page will still work, other pages will show config prompt
        st.session_state.config = None
        st.session_state.controller = None
    except Exception as e:
        # Other errors - show warning but allow linter to work
        st.session_state.config = None
        st.session_state.controller = None
        st.sidebar.warning(f"Config load failed: {e}")

# Title
st.title("🔌 Strimzi Ops Platform")
st.markdown("Monitor and Control your Kafka Connect deployments")

PAGES = ["Dashboard", "Monitor", "Control"]
if "page" not in st.session_state:
    st.session_state.page = "Dashboard"


def navigate(to: str, **state) -> None:
    """Switch sidebar page and optionally stash related session state."""
    st.session_state.page = to
    for key, value in state.items():
        st.session_state[key] = value
    st.rerun()


# Sidebar navigation (session-state driven so Dashboard can deep-link)
selection = st.sidebar.radio(
    "Navigation",
    PAGES,
    index=PAGES.index(st.session_state.page),
)
if selection != st.session_state.page:
    st.session_state.page = selection
page = st.session_state.page

# Dashboard Page
if page == "Dashboard":
    st.header("📈 Dashboard")
    st.markdown("Overview of your Kafka Connect deployment")

    # Check if config is available
    if st.session_state.config is None or st.session_state.controller is None:
        st.warning("Configuration Required")
        st.info(
            """
        To use the Dashboard feature, you need to create a `secrets.toml` file with your Kafka configuration.
        """
        )
        st.stop()

    try:

        def _truncate_error(trace: str | None, limit: int = 200) -> str:
            text = trace or "Unknown error"
            return text if len(text) <= limit else text[:limit] + "..."

        cluster_info = st.session_state.controller.get_cluster_info()
        all_info = st.session_state.controller.get_all_connectors_status()
        plugins = st.session_state.controller.get_connector_plugins()

        if all_info:
            # Aggregate status counts
            total_connectors = len(all_info)
            running_connectors = 0
            failed_connectors = 0
            total_tasks = 0
            running_tasks = 0

            status_distribution: dict[str, int] = {}

            for _name, info in all_info.items():
                status = info.get("status", {})
                connector_state = status.get("connector", {}).get("state", "UNKNOWN")

                if connector_state == "RUNNING":
                    running_connectors += 1
                elif connector_state == "FAILED":
                    failed_connectors += 1

                status_distribution[connector_state] = (
                    status_distribution.get(connector_state, 0) + 1
                )

                tasks = status.get("tasks", [])
                total_tasks += len(tasks)
                for t in tasks:
                    if t.get("state") == "RUNNING":
                        running_tasks += 1

            # Cluster Info Header
            c1, c2, c3 = st.columns(3)
            c1.info(f"**Connect Version:** {cluster_info.get('version', 'Unknown')}")
            c2.info(f"**Kafka Cluster ID:** {cluster_info.get('kafka_cluster_id', 'Unknown')}")
            c3.info(f"**Plugins Available:** {len(plugins)}")

            # Summary Metrics
            st.divider()
            m1, m2, m3, m4 = st.columns(4)
            m1.metric("Total Connectors", total_connectors)
            m2.metric("Running", running_connectors)
            m3.metric(
                "Failed",
                failed_connectors,
                delta=-failed_connectors if failed_connectors > 0 else 0,
                delta_color="inverse",
            )
            m4.metric("Tasks Running", f"{running_tasks}/{total_tasks}")

            col1, col2 = st.columns(2)

            with col1:
                st.subheader("Connector Status Distribution")
                st.caption("Snapshot and CDC progress live on Monitor.")
                st.bar_chart({"Count": status_distribution})
                if st.button("Open Monitor →", key="dash_to_monitor", use_container_width=True):
                    navigate("Monitor")

            with col2:
                st.subheader("Failed Connectors/Tasks")
                issues = []
                for name, info in all_info.items():
                    status = info.get("status", {})
                    connector_state = status.get("connector", {}).get("state", "UNKNOWN")
                    tasks = status.get("tasks", [])

                    if connector_state == "FAILED":
                        issues.append(
                            {
                                "Name": name,
                                "Type": "Connector",
                                "Error": _truncate_error(status.get("connector", {}).get("trace")),
                            }
                        )

                    for t in tasks:
                        if t.get("state") == "FAILED":
                            issues.append(
                                {
                                    "Name": f"{name} (Task {t.get('id')})",
                                    "Type": "Task",
                                    "Error": _truncate_error(t.get("trace")),
                                }
                            )

                if issues:
                    st.table(issues)
                    if st.button("Manage failed connectors →", key="dash_failed_to_control"):
                        first = issues[0]["Name"].split(" (Task")[0]
                        navigate("Control", focus_connector=first)
                else:
                    st.success("✅ No failures detected")

            st.divider()
            st.subheader("All Connectors Summary")
            st.caption("Open a connector to manage it on the Control page.")
            header = st.columns([3, 1, 1, 1, 1])
            header[0].markdown("**Name**")
            header[1].markdown("**Type**")
            header[2].markdown("**Status**")
            header[3].markdown("**Tasks**")
            header[4].markdown("**Actions**")

            for name, info in all_info.items():
                status = info.get("status", {})
                tasks = status.get("tasks", [])
                running = sum(1 for t in tasks if t.get("state") == "RUNNING")
                row = st.columns([3, 1, 1, 1, 1])
                row[0].write(name)
                row[1].write(info.get("info", {}).get("type", "unknown"))
                row[2].write(status.get("connector", {}).get("state", "UNKNOWN"))
                row[3].write(f"{running}/{len(tasks)}")
                if row[4].button("Manage →", key=f"dash_manage_{name}"):
                    navigate("Control", focus_connector=name)

        else:
            st.info("No connectors found")

    except Exception as e:
        st.error(f"Failed to fetch dashboard data: {e}")

# Monitor Page
elif page == "Monitor":
    st.header("📡 Real-time Snapshot Monitoring")
    st.markdown("Track Debezium snapshot progress via notification events")

    # Check if config is available
    if st.session_state.config is None:
        st.warning("⚙️ Configuration Required")
        st.info(
            """
        To use the Monitor feature, you need to create a `secrets.toml` file with your Kafka configuration.

        **Example secrets.toml:**
        ```toml
        [kafka]
        bootstrap_servers = "localhost:9092"
        connect_url = "http://localhost:8083"

        [storage]
        type = "s3"
        endpoint_url = "http://localhost:3900"
        access_key = "YOUR_ACCESS_KEY"
        secret_key = "YOUR_SECRET_KEY"
        bucket = "warehouse"
        ```

        After creating the file, refresh the page.
        """
        )
        st.stop()

    # Monitor controls
    col1, col2 = st.columns([3, 1])

    with col1:
        notification_topic = st.text_input("Notification Topic", value="debezium.notifications")

    with col2:
        monitor_duration = st.number_input(
            "Duration (seconds)", min_value=10, max_value=300, value=60
        )

    if st.button("Start Monitoring"):
        try:
            from strimzi_ops.monitor import DebeziumNotificationMonitor, SnapshotTracker

            monitor = DebeziumNotificationMonitor(
                st.session_state.config.kafka_bootstrap_servers, notification_topic
            )
            tracker = SnapshotTracker()

            monitor.start()

            progress_container = st.empty()
            status_container = st.empty()

            def display_notification(notification):
                tracker.process_notification(notification)
                snapshots = tracker.get_all_snapshots()

                with status_container.container():
                    st.subheader("Snapshot Status")
                    for connector, snapshot_info in snapshots.items():
                        status = snapshot_info.get("status", "UNKNOWN")
                        progress = snapshot_info.get("progress", 0)

                        st.markdown(f"**{connector}**")
                        st.progress(progress / 100)
                        st.text(f"Status: {status} - Progress: {progress}%")

            with st.spinner(f"Monitoring for {monitor_duration} seconds..."):
                monitor.consume_notifications(
                    callback=display_notification, duration_seconds=monitor_duration
                )

            st.success("Monitoring completed")

        except Exception as e:
            st.error(f"Monitoring failed: {e}")

# Control Page
elif page == "Control":
    st.header("🎮 Connector Control")
    st.markdown("Manage your Kafka Connect connectors")

    # Check if config is available
    if st.session_state.config is None or st.session_state.controller is None:
        st.warning("⚙️ Configuration Required")
        st.info(
            """
        To use the Control feature, you need to create a `secrets.toml` file with your Kafka configuration.

        **Example secrets.toml:**
        ```toml
        [kafka]
        bootstrap_servers = "localhost:9092"
        connect_url = "http://localhost:8083"

        [storage]
        type = "s3"
        endpoint_url = "http://localhost:3900"
        access_key = "YOUR_ACCESS_KEY"
        secret_key = "YOUR_SECRET_KEY"
        bucket = "warehouse"
        ```

        After creating the file, refresh the page.
        """
        )
        st.stop()

    # List connectors
    try:
        all_info = st.session_state.controller.get_all_connectors_status()

        if all_info:
            focus = st.session_state.pop("focus_connector", None)
            if focus and focus in all_info:
                st.info(f"Focused from Dashboard: **{focus}**")
                # Show focused connector first
                all_info = {
                    focus: all_info[focus],
                    **{k: v for k, v in all_info.items() if k != focus},
                }

            # Table Header
            header_cols = st.columns([2, 1, 1, 4])
            header_cols[0].markdown("**Connector Name**")
            header_cols[1].markdown("**State**")
            header_cols[2].markdown("**Tasks**")
            header_cols[3].markdown("**Actions**")
            st.divider()

            for name, info in all_info.items():
                status = info.get("status", {})
                connector_state = status.get("connector", {}).get("state", "UNKNOWN")
                tasks = status.get("tasks", [])
                running_tasks = sum(1 for t in tasks if t.get("state") == "RUNNING")
                total_tasks = len(tasks)

                cols = st.columns([2, 1, 1, 4])

                # Name
                cols[0].write(name)

                # State with color
                state_color = (
                    "green"
                    if connector_state == "RUNNING"
                    else "orange"
                    if connector_state == "PAUSED"
                    else "red"
                )
                cols[1].markdown(f":{state_color}[{connector_state}]")

                # Tasks
                cols[2].write(f"{running_tasks}/{total_tasks}")

                # Actions
                btn_cols = cols[3].columns(7)

                # Resume
                if btn_cols[0].button("▶️", key=f"res_{name}", help="Resume"):
                    try:
                        st.session_state.controller.resume_connector(name)
                        st.success(f"Resumed {name}")
                        st.rerun()
                    except Exception as e:
                        st.error(f"Failed to resume: {e}")

                # Pause
                if btn_cols[1].button("⏸️", key=f"pau_{name}", help="Pause"):
                    try:
                        st.session_state.controller.pause_connector(name)
                        st.success(f"Paused {name}")
                        st.rerun()
                    except Exception as e:
                        st.error(f"Failed to pause: {e}")

                # Restart
                if btn_cols[2].button("🔄", key=f"res_all_{name}", help="Restart"):
                    try:
                        st.session_state.controller.restart_connector(name)
                        st.success(f"Restarted {name}")
                        st.rerun()
                    except Exception as e:
                        st.error(f"Failed to restart: {e}")

                # Snapshot
                if btn_cols[3].button("📸", key=f"snap_{name}", help="Trigger Snapshot"):
                    st.session_state.triggering_snapshot = name

                # Export to Strimzi YAML
                if btn_cols[4].button("📄", key=f"yaml_{name}", help="Export to Strimzi YAML"):
                    st.session_state.exporting_yaml = name

                # View Logs
                if btn_cols[5].button("📋", key=f"logs_{name}", help="View Logs"):
                    st.session_state.viewing_logs = name

                # Edit Config
                if btn_cols[6].button("⚙️", key=f"edit_{name}", help="Edit Configuration"):
                    st.session_state.editing_connector = name

            # Snapshot Trigger UI
            if "triggering_snapshot" in st.session_state:
                snap_name = st.session_state.triggering_snapshot
                st.divider()
                st.subheader(f"📸 Trigger Snapshot: {snap_name}")

                col1, col2 = st.columns(2)
                with col1:
                    snap_type = st.selectbox("Snapshot Type", ["incremental", "blocking"])
                with col2:
                    snap_tables = st.text_input(
                        "Tables (comma-separated, optional)", help="e.g. public.users,public.orders"
                    )

                c1, c2 = st.columns([1, 5])
                if c1.button("Execute Snapshot", type="primary"):
                    try:
                        tables_list = (
                            [t.strip() for t in snap_tables.split(",")] if snap_tables else None
                        )
                        result = st.session_state.controller.trigger_snapshot(
                            snap_name, snap_type, tables_list
                        )
                        if result["status"] == "success":
                            st.success(f"Snapshot triggered: {result['message']}")
                        else:
                            st.warning(f"Fallback triggered: {result['message']}")
                        del st.session_state.triggering_snapshot
                        st.rerun()
                    except Exception as e:
                        st.error(f"Failed to trigger snapshot: {e}")

                if c2.button("Cancel Snapshot"):
                    del st.session_state.triggering_snapshot
                    st.rerun()

            # Log View
            if "viewing_logs" in st.session_state:
                from strimzi_ops.k8s import fetch_logs

                log_connector = st.session_state.viewing_logs
                st.divider()
                st.subheader(f"📋 Logs for Cluster: {st.session_state.config.connect_cluster_name}")
                st.info(f"Showing recent logs filtered for connector: {log_connector}")

                c1, c2 = st.columns([1, 5])
                refresh_logs = c1.button("Refresh Logs", type="primary")
                if c2.button("Close Logs"):
                    del st.session_state.viewing_logs
                    st.session_state.pop("log_cache", None)
                    st.session_state.pop("log_cache_connector", None)
                    st.rerun()

                cache_stale = st.session_state.get("log_cache_connector") != log_connector
                if refresh_logs or cache_stale or "log_cache" not in st.session_state:
                    st.session_state.log_cache = fetch_logs(
                        st.session_state.config.connect_cluster_name,
                        lines=200,
                        filter_text=log_connector,
                    )
                    st.session_state.log_cache_connector = log_connector

                st.code(st.session_state.log_cache or "No logs available")

            # Export YAML view
            if "exporting_yaml" in st.session_state:
                export_name = st.session_state.exporting_yaml
                st.divider()
                st.subheader(f"Strimzi KafkaConnector YAML: {export_name}")

                try:
                    cluster_name = st.session_state.config.connect_cluster_name
                    strimzi_yaml = st.session_state.controller.to_strimzi_yaml(
                        export_name, cluster_name
                    )
                    st.code(strimzi_yaml, language="yaml")
                    st.download_button(
                        "Download YAML", strimzi_yaml, f"{export_name}.yaml", "text/yaml"
                    )
                    if st.button("Close Preview"):
                        del st.session_state.exporting_yaml
                        st.rerun()
                except Exception as e:
                    st.error(f"Failed to generate YAML: {e}")

            # Configuration editor
            if "editing_connector" in st.session_state:
                edit_name = st.session_state.editing_connector
                st.divider()
                st.subheader(f"Edit Configuration: {edit_name}")

                try:
                    config = st.session_state.controller.get_connector_config(edit_name)
                    # Filter out internal fields if necessary, but Connect API usually returns what's needed

                    config_json = st.text_area(
                        "JSON Configuration",
                        value=json.dumps(config, indent=2),
                        height=400,
                        key=f"config_area_{edit_name}",
                    )

                    c1, c2 = st.columns([1, 5])
                    if c1.button("Update", type="primary"):
                        try:
                            new_config = json.loads(config_json)

                            # Validate before update. Connect API configs omit "name",
                            # so inject the connector name for schema/lint checks.
                            from strimzi_ops.validator import ConnectorValidator

                            validator = ConnectorValidator()
                            validation_results = validator.validate_config(
                                new_config, connector_name=edit_name
                            )

                            if not validation_results["valid"]:
                                st.error("Configuration is invalid:")
                                st.text(validation_results["formatted"])
                                if st.button("Update Anyway"):
                                    st.session_state.controller.update_connector(
                                        edit_name, new_config
                                    )
                                    st.success(
                                        f"Configuration for {edit_name} updated (ignoring errors)"
                                    )
                                    del st.session_state.editing_connector
                                    st.rerun()
                            else:
                                st.session_state.controller.update_connector(edit_name, new_config)
                                st.success(f"Configuration for {edit_name} updated")
                                del st.session_state.editing_connector
                                st.rerun()
                        except Exception as e:
                            st.error(f"Failed to update: {e}")

                    if c2.button("Cancel"):
                        del st.session_state.editing_connector
                        st.rerun()

                except Exception as e:
                    st.error(f"Failed to fetch configuration: {e}")

        else:
            st.info("No connectors found")

            # Create new connector section
            with st.expander("➕ Create New Connector"):
                new_config = st.text_area(
                    "Connector Configuration (JSON)",
                    height=300,
                    placeholder='{\n  "name": "my-connector",\n  "config": {...}\n}',
                )

                if st.button("Create Connector"):
                    if new_config:
                        try:
                            config = json.loads(new_config)
                            st.session_state.controller.create_connector(config)
                            st.success("Connector created successfully")
                            st.rerun()
                        except Exception as e:
                            st.error(f"Failed to create connector: {e}")
                    else:
                        st.warning("Please provide a configuration")

    except Exception as e:
        st.error(f"Failed to fetch connectors: {e}")

# Footer
st.sidebar.markdown("---")
st.sidebar.markdown("### About")
st.sidebar.info("🔌 Strimzi Ops - Kafka Connect Management Platform")
st.sidebar.markdown("[Documentation](https://github.com) • [Report Issue](https://github.com)")
