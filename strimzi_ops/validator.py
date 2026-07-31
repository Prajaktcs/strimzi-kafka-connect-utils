"""Validator for Kafka Connect connector configurations using the linter."""

import json
from io import StringIO
from typing import Any

from pydantic import ValidationError
from ruamel.yaml import YAML

from .linter import ConnectorLinter, LintResult, Severity
from .models import get_model_for_class


class ConnectorValidator:
    """Validator for Kafka Connect connector configurations."""

    def __init__(self, linter_config_path: str = ".lintrc.toml"):
        """
        Initialize the validator.

        Args:
            linter_config_path: Path to linter configuration file
        """
        self.linter = ConnectorLinter(linter_config_path)

    @staticmethod
    def _with_name(config: dict[str, Any], connector_name: str | None) -> dict[str, Any]:
        """
        Ensure config has a name.

        Kafka Connect's GET /connectors/{name}/config response omits ``name``,
        so callers can supply it explicitly for validation.
        """
        if "name" in config or not connector_name:
            return config
        return {**config, "name": connector_name}

    @staticmethod
    def _parse_text(text: str, format: str) -> dict[str, Any]:
        """Parse connector config text into a dictionary."""
        yaml = YAML(typ="safe")

        if format == "json":
            parsed = json.loads(text)
        elif format == "yaml":
            parsed = yaml.load(StringIO(text))
        else:
            try:
                parsed = json.loads(text)
            except json.JSONDecodeError:
                parsed = yaml.load(StringIO(text))

        if not isinstance(parsed, dict):
            return {}
        return parsed

    def _validate(self, config: dict[str, Any], results: list[LintResult]) -> dict[str, Any]:
        """Internal validation logic combining linting and Pydantic."""
        connector_class = config.get("connector.class")
        if connector_class:
            model = get_model_for_class(connector_class)
            try:
                model(**config)
            except ValidationError as e:
                for error in e.errors():
                    loc = " -> ".join(str(x) for x in error["loc"])
                    results.append(
                        LintResult(
                            rule_id="pydantic-schema",
                            severity=Severity.ERROR,
                            message=f"{error['msg']} for field {loc}",
                            path=loc,
                        )
                    )

        summary = self.linter.get_summary(results)

        return {
            "valid": summary["errors"] == 0,
            "results": results,
            "summary": summary,
            "formatted": self.linter.format_results(results),
        }

    def validate_config(
        self, config: dict[str, Any], connector_name: str | None = None
    ) -> dict[str, Any]:
        """
        Validate connector configuration.

        Args:
            config: Connector configuration dictionary
            connector_name: Optional name to inject when config omits ``name``
                (e.g. Kafka Connect REST API config payloads)

        Returns:
            Dictionary with validation results
        """
        config = self._with_name(config, connector_name)
        results = self.linter.lint(config)
        return self._validate(config, results)

    def validate_text(
        self, text: str, format: str = "auto", connector_name: str | None = None
    ) -> dict[str, Any]:
        """
        Validate connector configuration from raw text (YAML or JSON).
        Supports comment-based lint directives.

        Args:
            text: Raw configuration text
            format: Format type - "yaml", "json", or "auto" (default)
            connector_name: Optional name to inject when config omits ``name``

        Returns:
            Dictionary with validation results
        """
        results = self.linter.lint_text(text, format=format)

        try:
            config = self._parse_text(text, format)
        except Exception:
            config = {}

        config = self._with_name(config, connector_name)
        return self._validate(config, results)

    def has_errors(self, results: list[LintResult]) -> bool:
        """
        Check if there are any errors in the results.

        Args:
            results: List of lint results

        Returns:
            True if there are errors
        """
        return any(r.severity == Severity.ERROR for r in results)

    def get_errors(self, results: list[LintResult]) -> list[LintResult]:
        """
        Get only error results.

        Args:
            results: List of lint results

        Returns:
            List of error results
        """
        return [r for r in results if r.severity == Severity.ERROR]

    def get_warnings(self, results: list[LintResult]) -> list[LintResult]:
        """
        Get only warning results.

        Args:
            results: List of lint results

        Returns:
            List of warning results
        """
        return [r for r in results if r.severity == Severity.WARNING]
