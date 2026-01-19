"""
Strimzi Ops Platform - Streamlit Application

A unified platform to validate, monitor, and control Kafka Connect deployments.
"""

import streamlit as st
import json
from strimzi_ops.validator import ConnectorValidator

# Page configuration
st.set_page_config(
    page_title="Strimzi Ops Platform",
    page_icon="🔌",
    layout="wide"
)

# Initialize session state
if 'config' not in st.session_state:
    try:
        from strimzi_ops.config import Config
        from strimzi_ops.control import ConnectorController

        st.session_state.config = Config()
        st.session_state.controller = ConnectorController(
            st.session_state.config.kafka_connect_url
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
st.markdown("Validate, Monitor, and Control your Kafka Connect deployments")

# Sidebar navigation
page = st.sidebar.selectbox(
    "Navigation",
    ["Linter", "Dashboard", "Monitor", "Control"]
)

# Linter Page (Primary Feature)
if page == "Linter":
    st.header("✅ Connector Configuration Linter")
    st.markdown("Lint your Kafka Connect connector configurations with configurable rules")

    # Show linter configuration info
    with st.expander("ℹ️  About the Linter"):
        st.markdown("""
        The linter checks your connector configurations for common issues and best practices.

        **Features:**
        - Configurable rules via `.lintrc.toml`
        - Per-connector rule exemptions
        - Severity levels: Error, Warning, Info
        - Inline rule disabling using comments (YAML/JSON)

        **Example: Disable specific rules with comments**
        ```yaml
        # lint-disable: naming-convention, sensitive-data
        name: my-connector
        connector.class: io.debezium.connector.postgresql.PostgresConnector
        tasks.max: 1
        ```

        **Supported comment directives:**
        - `# lint-disable: rule1, rule2` - Disable rules for this file
        - `# lint-disable-file: rule1` - Alternative syntax
        """)

    # Input method selection
    input_method = st.radio(
        "Input Method",
        ["YAML/JSON Editor", "Upload File"]
    )

    config_text = ""
    file_format = "auto"

    if input_method == "YAML/JSON Editor":
        config_text = st.text_area(
            "Connector Configuration (YAML or JSON)",
            height=300,
            placeholder='# Use YAML with comments to disable rules\n# lint-disable: naming-convention\nname: my-connector\nconnector.class: ...\n'
        )
    else:
        uploaded_file = st.file_uploader("Upload Configuration File", type=['json', 'yaml', 'yml'])
        if uploaded_file:
            config_text = uploaded_file.read().decode('utf-8')
            # Detect format from file extension
            if uploaded_file.name.endswith(('.yaml', '.yml')):
                file_format = "yaml"
                st.code(config_text, language='yaml')
            else:
                file_format = "json"
                st.code(config_text, language='json')

    if st.button("Lint Configuration"):
        if config_text:
            try:
                validator = ConnectorValidator()
                result = validator.validate_text(config_text, format=file_format)

                # Display summary
                col1, col2, col3 = st.columns(3)
                with col1:
                    if result['summary']['errors'] > 0:
                        st.metric("Errors", result['summary']['errors'], delta=None, delta_color="inverse")
                    else:
                        st.metric("Errors", 0)

                with col2:
                    st.metric("Warnings", result['summary']['warnings'])

                with col3:
                    st.metric("Info", result['summary']['info'])

                # Display results
                if result['valid']:
                    if result['summary']['warnings'] == 0 and result['summary']['info'] == 0:
                        st.success("✅ Configuration is perfect! No issues found.")
                    else:
                        st.success("✅ Configuration is valid (no errors)")
                else:
                    st.error("❌ Configuration has errors")

                # Show detailed results
                if result['results']:
                    st.subheader("Lint Results")

                    # Group by severity
                    errors = validator.get_errors(result['results'])
                    warnings = validator.get_warnings(result['results'])

                    if errors:
                        st.markdown("### ❌ Errors")
                        for lint_result in errors:
                            st.error(str(lint_result))

                    if warnings:
                        st.markdown("### ⚠️  Warnings")
                        for lint_result in warnings:
                            st.warning(str(lint_result))

            except (json.JSONDecodeError, ValueError) as e:
                st.error(f"Invalid configuration format: {e}")
        else:
            st.warning("Please provide a configuration to validate")

# Dashboard Page
elif page == "Dashboard":
    st.header("📊 Dashboard")
    st.markdown("Overview of your Kafka Connect deployment")

    # Check if config is available
    if st.session_state.config is None or st.session_state.controller is None:
        st.warning("⚙️ Configuration Required")
        st.info("""
        To use the Dashboard feature, you need to create a `secrets.toml` file with your Kafka configuration.

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
        """)
        st.stop()

    # Dashboard implementation coming soon
    st.info("📈 Dashboard implementation coming soon!")
    st.markdown("""
    **Planned Features:**
    - Connector health overview
    - Real-time metrics
    - Task status tracking
    - Error monitoring
    """)

# Monitor Page
elif page == "Monitor":
    st.header("📡 Real-time Snapshot Monitoring")
    st.markdown("Track Debezium snapshot progress via notification events")

    # Check if config is available
    if st.session_state.config is None:
        st.warning("⚙️ Configuration Required")
        st.info("""
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
        """)
        st.stop()

    # Monitor controls
    col1, col2 = st.columns([3, 1])

    with col1:
        notification_topic = st.text_input(
            "Notification Topic",
            value="debezium.notifications"
        )

    with col2:
        monitor_duration = st.number_input(
            "Duration (seconds)",
            min_value=10,
            max_value=300,
            value=60
        )

    if st.button("Start Monitoring"):
        try:
            from strimzi_ops.monitor import DebeziumNotificationMonitor, SnapshotTracker

            monitor = DebeziumNotificationMonitor(
                st.session_state.config.kafka_bootstrap_servers,
                notification_topic
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
                    callback=display_notification,
                    duration_seconds=monitor_duration
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
        st.info("""
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
        """)
        st.stop()

    # List connectors
    try:
        connectors = st.session_state.controller.list_connectors()

        if connectors:
            selected_connector = st.selectbox("Select Connector", connectors)

            if selected_connector:
                # Display connector status
                status = st.session_state.controller.get_connector_status(selected_connector)
                st.subheader(f"Status: {selected_connector}")
                st.json(status)

                # Control buttons
                col1, col2, col3, col4 = st.columns(4)

                with col1:
                    if st.button("▶️ Resume"):
                        try:
                            st.session_state.controller.resume_connector(selected_connector)
                            st.success(f"Resumed {selected_connector}")
                            st.rerun()
                        except Exception as e:
                            st.error(f"Failed to resume: {e}")

                with col2:
                    if st.button("⏸️ Pause"):
                        try:
                            st.session_state.controller.pause_connector(selected_connector)
                            st.success(f"Paused {selected_connector}")
                            st.rerun()
                        except Exception as e:
                            st.error(f"Failed to pause: {e}")

                with col3:
                    if st.button("🔄 Restart"):
                        try:
                            st.session_state.controller.restart_connector(selected_connector)
                            st.success(f"Restarted {selected_connector}")
                            st.rerun()
                        except Exception as e:
                            st.error(f"Failed to restart: {e}")

                with col4:
                    if st.button("📸 Trigger Snapshot"):
                        try:
                            result = st.session_state.controller.trigger_snapshot(selected_connector)
                            st.success(f"Snapshot triggered: {result}")
                        except Exception as e:
                            st.error(f"Failed to trigger snapshot: {e}")

                # Configuration editor
                st.subheader("Configuration")
                config = st.session_state.controller.get_connector_config(selected_connector)
                config_json = st.text_area(
                    "Edit Configuration",
                    value=json.dumps(config, indent=2),
                    height=300
                )

                if st.button("Update Configuration"):
                    try:
                        new_config = json.loads(config_json)
                        st.session_state.controller.update_connector(
                            selected_connector,
                            new_config
                        )
                        st.success("Configuration updated successfully")
                        st.rerun()
                    except Exception as e:
                        st.error(f"Failed to update configuration: {e}")

        else:
            st.info("No connectors found")

            # Create new connector section
            st.subheader("Create New Connector")
            new_config = st.text_area(
                "Connector Configuration (JSON)",
                height=300,
                placeholder='{\n  "name": "my-connector",\n  "config": {...}\n}'
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
