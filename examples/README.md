# Connector Configuration Examples

This directory contains example connector configurations for Strimzi Ops.

## Linter Features

The connector linter provides flexible validation with configurable rules:

### 1. Global Rule Configuration

Edit `.lintrc.toml` in the project root to disable rules globally or override severities:

```toml
# Disable rules for all connectors
disabled_rules = ["naming-convention"]

# Override severity levels
[rule_severities]
sensitive-data = "error"  # Upgrade from warning to error

# Per-connector exemptions
[connector_exemptions]
legacy-connector = ["naming-convention", "sensitive-data"]
```

### 2. Comment-Based Rule Disabling (Recommended)

Disable specific rules using comments in your YAML configuration files:

```yaml
# lint-disable: naming-convention, sensitive-data
name: my-connector
connector.class: io.debezium.connector.postgresql.PostgresConnector
database.password: hardcoded_password
```

**Supported comment directives:**
- `# lint-disable: rule1, rule2` - Disable rules for entire file
- `# lint-disable-file: rule1` - Alternative syntax

This approach keeps your actual connector configuration clean and works naturally with YAML files.

### 3. Built-in Rules

| Rule ID | Description | Default Severity |
|---------|-------------|------------------|
| `required-field` | Ensures required fields are present | Error |
| `naming-convention` | Validates connector naming conventions | Warning |
| `tasks-max-value` | Validates tasks.max configuration | Warning |
| `debezium-snapshot-mode` | Validates Debezium snapshot.mode values | Warning |
| `sensitive-data` | Detects hardcoded sensitive information | Warning |

### 4. Examples

**Standard YAML Configuration:**
```bash
cat debezium-postgres-connector.yaml
cat iceberg-sink-connector.yaml
```

**Legacy Configuration with Exemptions:**
```bash
cat legacy-connector-with-exemptions.yaml
```

**JSON Configurations (also supported):**
```bash
cat debezium-postgres-connector.json
cat iceberg-sink-connector.json
```

## Usage

### Via Streamlit UI

1. Start the application:
   ```bash
   uv run streamlit run app.py
   ```

2. Navigate to the "Linter" page

3. Paste your YAML or JSON configuration, or upload a file

4. Click "Lint Configuration"

### Via Python (YAML with comments)

```python
from strimzi_ops.validator import ConnectorValidator

# Load configuration from YAML file
with open("examples/debezium-postgres-connector.yaml") as f:
    config_text = f.read()

# Validate
validator = ConnectorValidator()
result = validator.validate_text(config_text, format="yaml")

print(result['formatted'])
print(f"Valid: {result['valid']}")
print(f"Summary: {result['summary']}")
```

### Via Python (Direct config dict)

```python
from strimzi_ops.validator import ConnectorValidator
import json

# Load configuration
with open("examples/debezium-postgres-connector.json") as f:
    config = json.load(f)

# Validate
validator = ConnectorValidator()
result = validator.validate_config(config)

print(result['formatted'])
```

## Best Practices

1. **Use YAML with Comments** for better readability and inline rule disabling:
   ```yaml
   # lint-disable: sensitive-data
   database.password: ${env:POSTGRES_PASSWORD}
   ```

2. **Use Environment Variables** for sensitive data:
   ```yaml
   database.password: ${env:POSTGRES_PASSWORD}
   ```

3. **Follow Naming Conventions**: Use lowercase with hyphens or underscores
   - Good: `postgres-source-connector`
   - Bad: `MyConnector!!123`

4. **Limit tasks.max**: Keep it reasonable (usually 1-10)

5. **Configure Exemptions Wisely**: Only disable rules when absolutely necessary

6. **Document Exemptions**: Use comments to explain why rules are disabled

## Severity Levels

- **Error** (❌): Configuration is invalid and will likely fail
- **Warning** (⚠️): Configuration works but may have issues
- **Info** (ℹ️): Informational messages for best practices

Errors must be fixed before deploying. Warnings should be reviewed but can be ignored if intentional.

## File Formats

Both JSON and YAML formats are supported:
- **YAML** (`.yaml`, `.yml`) - Recommended for comment-based rule disabling
- **JSON** (`.json`) - Standard Kafka Connect format

The linter automatically detects the format based on file extension or content.
