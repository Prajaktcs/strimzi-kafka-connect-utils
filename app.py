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
        st.session_state.controller = ConnectorController(st.session_state.config.kafka_connect_url)
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

# Sidebar navigation
page = st.sidebar.selectbox("Navigation", ["Dashboard", "Monitor", "Control"])

# Dashboard Page
if page == "Dashboard":
    st.header("Dashboard")
    st.markdown("Overview of your Kafka Connect deployment")

    # Check if config is available
    if st.session_state.config is None or st.session_state.controller is None:
        st.warning("Configuration Required")
        st.info(
            """
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
        """
        )
        st.stop()

    # Dashboard implementation coming soon
    st.info("📈 Dashboard implementation coming soon!")
    st.markdown(
        """
    **Planned Features:**
    - Connector health overview
    - Real-time metrics
    - Task status tracking
    - Error monitoring
    """
    )

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
                btn_cols = cols[3].columns(5)

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
                    try:
                        result = st.session_state.controller.trigger_snapshot(name)
                        st.success(f"Snapshot triggered for {name}")
                    except Exception as e:
                        st.error(f"Failed to trigger snapshot: {e}")

                # Edit Config
                if btn_cols[4].button("⚙️", key=f"edit_{name}", help="Edit Configuration"):
                    st.session_state.editing_connector = name

            # Configuration editor (shows below table when a connector is selected for editing)
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
