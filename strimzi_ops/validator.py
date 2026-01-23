"""Validator for Kafka Connect connector configurations using the linter."""

from typing import Any

from .linter import ConnectorLinter, LintResult, Severity


class ConnectorValidator:
    """Validator for Kafka Connect connector configurations."""

    def __init__(self, linter_config_path: str = ".lintrc.toml"):
        """
        Initialize the validator.

        Args:
            linter_config_path: Path to linter configuration file
        """
        self.linter = ConnectorLinter(linter_config_path)

    def validate_config(self, config: dict[str, Any]) -> dict[str, Any]:
        """
        Validate connector configuration.

        Args:
            config: Connector configuration dictionary

        Returns:
            Dictionary with validation results
        """
        results = self.linter.lint(config)
        summary = self.linter.get_summary(results)

        return {
            "valid": summary["errors"] == 0,
            "results": results,
            "summary": summary,
            "formatted": self.linter.format_results(results),
        }

    def validate_text(self, text: str, format: str = "auto") -> dict[str, Any]:
        """
        Validate connector configuration from raw text (YAML or JSON).
        Supports comment-based lint directives.

        Args:
            text: Raw configuration text
            format: Format type - "yaml", "json", or "auto" (default)

        Returns:
            Dictionary with validation results
        """
        results = self.linter.lint_text(text, format=format)
        summary = self.linter.get_summary(results)

        return {
            "valid": summary["errors"] == 0,
            "results": results,
            "summary": summary,
            "formatted": self.linter.format_results(results),
        }

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
