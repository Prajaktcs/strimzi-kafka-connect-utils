"""Command-line interface for Strimzi Ops tools."""

import argparse
import sys
from pathlib import Path

from .validator import ConnectorValidator


def lint_command(args: argparse.Namespace) -> int:
    """
    Run the linter on connector configuration files.

    Args:
        args: Parsed command-line arguments

    Returns:
        Exit code (0 for success, 1 for errors)
    """
    # Read configuration file
    config_path = Path(args.file)
    if not config_path.exists():
        print(f"Error: File not found: {config_path}", file=sys.stderr)
        return 1

    try:
        with open(config_path) as f:
            config_text = f.read()
    except Exception as e:
        print(f"Error reading file: {e}", file=sys.stderr)
        return 1

    # Determine format
    format_type = args.format
    if format_type == "auto":
        if config_path.suffix in [".yaml", ".yml"]:
            format_type = "yaml"
        elif config_path.suffix == ".json":
            format_type = "json"

    # Initialize validator with linter config
    linter_config = args.config if args.config else ".lintrc.toml"
    validator = ConnectorValidator(linter_config_path=linter_config)

    # Run linter
    try:
        result = validator.validate_text(config_text, format=format_type)
    except Exception as e:
        print(f"Error parsing configuration: {e}", file=sys.stderr)
        return 1

    # Display results
    if args.json:
        import json

        output = {
            "valid": result["valid"],
            "summary": result["summary"],
            "results": [
                {
                    "rule_id": r.rule_id,
                    "severity": r.severity.value,
                    "message": r.message,
                    "path": r.path,
                }
                for r in result["results"]
            ],
        }
        print(json.dumps(output, indent=2))
    else:
        # Human-readable output
        if result["results"]:
            print(result["formatted"])
        else:
            print("✅ No issues found")

        print()
        summary = result["summary"]
        print(
            f"Summary: {summary['errors']} errors, {summary['warnings']} warnings, {summary['info']} info"
        )

    # Exit with error code if there are errors
    if not result["valid"] or (args.strict and result["summary"]["warnings"] > 0):
        return 1

    return 0


def main() -> int:
    """Main entry point for the CLI."""
    parser = argparse.ArgumentParser(
        description="Strimzi Ops - Kafka Connect management tools",
        formatter_class=argparse.RawDescriptionHelpFormatter,
        epilog="""
Examples:
  # Lint a YAML configuration
  strimzi-lint examples/debezium-postgres-connector.yaml

  # Lint with custom linter config
  strimzi-lint -c custom-lint.toml connector.yaml

  # Output in JSON format
  strimzi-lint --json connector.yaml

  # Strict mode (warnings also cause failure)
  strimzi-lint --strict connector.yaml
""",
    )

    subparsers = parser.add_subparsers(dest="command", help="Available commands")

    # Lint command
    lint_parser = subparsers.add_parser(
        "lint",
        help="Lint a connector configuration file",
        description="Validate Kafka Connect connector configurations against linting rules",
    )
    lint_parser.add_argument("file", help="Path to the connector configuration file (YAML or JSON)")
    lint_parser.add_argument(
        "-c",
        "--config",
        help="Path to linter configuration file (default: .lintrc.toml)",
        default=None,
    )
    lint_parser.add_argument(
        "-f",
        "--format",
        choices=["auto", "yaml", "json"],
        default="auto",
        help="Configuration file format (default: auto-detect)",
    )
    lint_parser.add_argument("--json", action="store_true", help="Output results in JSON format")
    lint_parser.add_argument(
        "--strict", action="store_true", help="Treat warnings as errors (exit code 1)"
    )

    # If no subcommand, default to lint for backward compatibility
    args = parser.parse_args()

    if args.command == "lint":
        return lint_command(args)
    elif not args.command:
        # No subcommand provided, show help
        parser.print_help()
        return 0
    else:
        parser.print_help()
        return 1


if __name__ == "__main__":
    sys.exit(main())
