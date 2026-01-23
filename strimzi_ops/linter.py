"""Linter for Kafka Connect connector configurations with rule management."""

import json
import re
from collections.abc import Callable
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from typing import Any

import tomli
from ruamel.yaml import YAML


class Severity(str, Enum):
    """Rule severity levels."""

    ERROR = "error"
    WARNING = "warning"
    INFO = "info"


@dataclass
class LintResult:
    """Result of a lint check."""

    rule_id: str
    severity: Severity
    message: str
    path: str | None = None

    def __str__(self) -> str:
        prefix = {Severity.ERROR: "❌", Severity.WARNING: "⚠️ ", Severity.INFO: "ℹ️ "}[self.severity]
        location = f" at {self.path}" if self.path else ""
        return f"{prefix} [{self.rule_id}] {self.message}{location}"


@dataclass
class Rule:
    """Lint rule definition."""

    id: str
    name: str
    description: str
    severity: Severity
    check: Callable[[dict[str, Any]], list[LintResult]]

    def __call__(self, config: dict[str, Any]) -> list[LintResult]:
        """Execute the rule check."""
        return self.check(config)


class LinterConfig:
    """Configuration for the linter."""

    def __init__(self, config_path: str | None = None):
        """
        Initialize linter configuration.

        Args:
            config_path: Path to .lintrc.toml file
        """
        self.disabled_rules: set[str] = set()
        self.rule_severities: dict[str, Severity] = {}
        self.connector_exemptions: dict[str, set[str]] = {}

        if config_path and Path(config_path).exists():
            self._load_config(config_path)

    def _load_config(self, config_path: str) -> None:
        """Load configuration from TOML file."""
        with open(config_path, "rb") as f:
            config = tomli.load(f)

        # Global disabled rules
        if "disabled_rules" in config:
            self.disabled_rules = set(config["disabled_rules"])

        # Rule severity overrides
        if "rule_severities" in config:
            self.rule_severities = {k: Severity(v) for k, v in config["rule_severities"].items()}

        # Per-connector exemptions
        if "connector_exemptions" in config:
            self.connector_exemptions = {
                k: set(v) for k, v in config["connector_exemptions"].items()
            }

    def is_rule_enabled(self, rule_id: str, connector_name: str | None = None) -> bool:
        """
        Check if a rule is enabled.

        Args:
            rule_id: Rule identifier
            connector_name: Optional connector name for per-connector checks

        Returns:
            True if rule should be checked
        """
        # Check global disable
        if rule_id in self.disabled_rules:
            return False

        # Check connector-specific exemptions
        if connector_name and connector_name in self.connector_exemptions:
            if rule_id in self.connector_exemptions[connector_name]:
                return False

        return True

    def get_severity(self, rule_id: str, default: Severity) -> Severity:
        """
        Get severity for a rule.

        Args:
            rule_id: Rule identifier
            default: Default severity

        Returns:
            Configured or default severity
        """
        return self.rule_severities.get(rule_id, default)


class ConnectorLinter:
    """Linter for Kafka Connect connector configurations."""

    def __init__(self, config_path: str | None = ".lintrc.toml"):
        """
        Initialize the linter.

        Args:
            config_path: Path to linter configuration file
        """
        self.config = LinterConfig(config_path)
        self.rules: list[Rule] = []
        self._register_builtin_rules()

    def _register_builtin_rules(self) -> None:
        """Register built-in lint rules."""

        # Required fields rule
        def check_required_fields(config: dict[str, Any]) -> list[LintResult]:
            results = []
            required = ["name", "connector.class"]
            for field in required:
                if field not in config:
                    results.append(
                        LintResult(
                            rule_id="required-field",
                            severity=Severity.ERROR,
                            message=f"Missing required field: {field}",
                            path=field,
                        )
                    )
            return results

        self.rules.append(
            Rule(
                id="required-field",
                name="Required Fields",
                description="Ensure required fields are present",
                severity=Severity.ERROR,
                check=check_required_fields,
            )
        )

        # Naming convention rule
        def check_naming_convention(config: dict[str, Any]) -> list[LintResult]:
            results: list[LintResult] = []
            if "name" in config:
                name = config["name"]
                if not isinstance(name, str):
                    return results

                # Check for valid characters
                if not all(c.isalnum() or c in "-_" for c in name):
                    results.append(
                        LintResult(
                            rule_id="naming-convention",
                            severity=Severity.WARNING,
                            message=f"Connector name '{name}' contains invalid characters",
                            path="name",
                        )
                    )

                # Check length
                if len(name) > 64:
                    results.append(
                        LintResult(
                            rule_id="naming-convention",
                            severity=Severity.WARNING,
                            message=f"Connector name '{name}' is too long (max 64 characters)",
                            path="name",
                        )
                    )

            return results

        self.rules.append(
            Rule(
                id="naming-convention",
                name="Naming Convention",
                description="Validate connector naming conventions",
                severity=Severity.WARNING,
                check=check_naming_convention,
            )
        )

        # Tasks max validation
        def check_tasks_max(config: dict[str, Any]) -> list[LintResult]:
            results = []
            if "tasks.max" in config:
                try:
                    tasks = int(config["tasks.max"])
                    if tasks < 1:
                        results.append(
                            LintResult(
                                rule_id="tasks-max-value",
                                severity=Severity.ERROR,
                                message="tasks.max must be at least 1",
                                path="tasks.max",
                            )
                        )
                    elif tasks > 10:
                        results.append(
                            LintResult(
                                rule_id="tasks-max-value",
                                severity=Severity.WARNING,
                                message=f"tasks.max is {tasks}, which may be too high",
                                path="tasks.max",
                            )
                        )
                except (ValueError, TypeError):
                    results.append(
                        LintResult(
                            rule_id="tasks-max-value",
                            severity=Severity.ERROR,
                            message="tasks.max must be a valid integer",
                            path="tasks.max",
                        )
                    )

            return results

        self.rules.append(
            Rule(
                id="tasks-max-value",
                name="Tasks Max Value",
                description="Validate tasks.max configuration",
                severity=Severity.WARNING,
                check=check_tasks_max,
            )
        )

        # Debezium snapshot mode
        def check_snapshot_mode(config: dict[str, Any]) -> list[LintResult]:
            results = []
            connector_class = config.get("connector.class", "")

            if "debezium" in connector_class.lower():
                if "snapshot.mode" in config:
                    mode = config["snapshot.mode"]
                    valid_modes = ["initial", "always", "never", "when_needed", "schema_only"]

                    if mode not in valid_modes:
                        results.append(
                            LintResult(
                                rule_id="debezium-snapshot-mode",
                                severity=Severity.WARNING,
                                message=f"Unknown snapshot.mode '{mode}'. Valid: {', '.join(valid_modes)}",
                                path="snapshot.mode",
                            )
                        )

            return results

        self.rules.append(
            Rule(
                id="debezium-snapshot-mode",
                name="Debezium Snapshot Mode",
                description="Validate Debezium snapshot.mode values",
                severity=Severity.WARNING,
                check=check_snapshot_mode,
            )
        )

        # Sensitive data detection
        def check_sensitive_data(config: dict[str, Any]) -> list[LintResult]:
            results = []
            sensitive_patterns = ["password", "secret", "key", "token"]

            for key, value in config.items():
                if any(pattern in key.lower() for pattern in sensitive_patterns):
                    if (
                        isinstance(value, str)
                        and len(value) > 0
                        and value not in ["${env:VAR}", "REPLACE_ME"]
                    ):
                        results.append(
                            LintResult(
                                rule_id="sensitive-data",
                                severity=Severity.WARNING,
                                message=f"Potential sensitive data in '{key}'. Consider using environment variables",
                                path=key,
                            )
                        )

            return results

        self.rules.append(
            Rule(
                id="sensitive-data",
                name="Sensitive Data Detection",
                description="Detect hardcoded sensitive information",
                severity=Severity.WARNING,
                check=check_sensitive_data,
            )
        )

    def _parse_lint_directives(self, text: str) -> set[str]:
        """
        Parse lint directives from comments in YAML/JSON text.

        Supports patterns:
        - # lint-disable: rule1, rule2
        - # lint-disable rule1, rule2
        - # lint-disable: rule1
        - # lint-disable-file: rule1, rule2

        Args:
            text: Raw configuration text with comments

        Returns:
            Set of disabled rule IDs
        """
        disabled_rules = set()

        # Match comment patterns for disabling rules
        # Note: Use [ \t] instead of \s to avoid matching newlines
        patterns = [
            r"#[ \t]*lint-disable(?:-file)?:[ \t]*([\w ,-]+)",
            r"#[ \t]*lint-disable(?:-file)?[ \t]+([\w ,-]+)",
        ]

        for pattern in patterns:
            matches = re.finditer(pattern, text, re.IGNORECASE)
            for match in matches:
                rules_str = match.group(1)
                # Split by comma and strip whitespace
                rules = [r.strip() for r in rules_str.split(",")]
                disabled_rules.update(rules)

        return disabled_rules

    def lint_text(self, text: str, format: str = "auto") -> list[LintResult]:
        """
        Lint a connector configuration from raw text.

        Args:
            text: Raw configuration text (YAML or JSON)
            format: Format type - "yaml", "json", or "auto" (default)

        Returns:
            List of lint results
        """
        # Parse lint directives from comments
        disabled_from_comments = self._parse_lint_directives(text)

        # Parse the configuration
        config = None
        if format == "auto":
            # Try JSON first, then YAML
            try:
                config = json.loads(text)
            except json.JSONDecodeError:
                try:
                    yaml = YAML()
                    from io import StringIO

                    stream = StringIO(text)
                    config = yaml.load(stream)
                except Exception:
                    raise ValueError("Could not parse configuration as JSON or YAML") from None
        elif format == "json":
            config = json.loads(text)
        elif format == "yaml":
            yaml = YAML()
            from io import StringIO

            stream = StringIO(text)
            config = yaml.load(stream)
        else:
            raise ValueError(f"Unknown format: {format}")

        # Lint with comment-based disabled rules
        return self.lint(config, disabled_inline=disabled_from_comments)

    def lint(
        self, config: dict[str, Any], disabled_inline: set[str] | None = None
    ) -> list[LintResult]:
        """
        Lint a connector configuration.

        Args:
            config: Connector configuration dictionary
            disabled_inline: Set of rule IDs to disable (from comments or other sources)

        Returns:
            List of lint results
        """
        results = []
        connector_name = config.get("name")

        # Use provided disabled rules or empty set
        if disabled_inline is None:
            disabled_inline = set()

        for rule in self.rules:
            # Skip if rule is disabled
            if rule.id in disabled_inline:
                continue

            if not self.config.is_rule_enabled(rule.id, connector_name):
                continue

            # Run rule check
            rule_results = rule(config)

            # Apply severity overrides
            for result in rule_results:
                result.severity = self.config.get_severity(rule.id, result.severity)
                results.append(result)

        return results

    def get_summary(self, results: list[LintResult]) -> dict[str, int]:
        """
        Get summary of lint results.

        Args:
            results: List of lint results

        Returns:
            Dictionary with counts by severity
        """
        summary = {
            "errors": sum(1 for r in results if r.severity == Severity.ERROR),
            "warnings": sum(1 for r in results if r.severity == Severity.WARNING),
            "info": sum(1 for r in results if r.severity == Severity.INFO),
        }
        return summary

    def format_results(self, results: list[LintResult]) -> str:
        """
        Format lint results as a string.

        Args:
            results: List of lint results

        Returns:
            Formatted string
        """
        if not results:
            return "✅ No issues found"

        output = []
        for result in sorted(results, key=lambda r: (r.severity.value, r.rule_id)):
            output.append(str(result))

        summary = self.get_summary(results)
        output.append("")
        output.append(
            f"Summary: {summary['errors']} errors, {summary['warnings']} warnings, {summary['info']} info"
        )

        return "\n".join(output)
