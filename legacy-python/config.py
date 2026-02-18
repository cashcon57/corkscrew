"""Configuration management for Corkscrew."""

import json
from pathlib import Path

CONFIG_PATH = Path.home() / ".config" / "corkscrew" / "config.json"


def get_config() -> dict[str, str]:
    """Load configuration from disk."""
    if not CONFIG_PATH.exists():
        return {}
    return json.loads(CONFIG_PATH.read_text(encoding="utf-8"))


def save_config(config: dict[str, str]):
    """Save configuration to disk."""
    CONFIG_PATH.parent.mkdir(parents=True, exist_ok=True)
    CONFIG_PATH.write_text(json.dumps(config, indent=2) + "\n", encoding="utf-8")


def set_config_value(key: str, value: str):
    """Set a single configuration value."""
    config = get_config()
    config[key] = value
    save_config(config)


def get_config_value(key: str, default: str | None = None) -> str | None:
    """Get a single configuration value."""
    return get_config().get(key, default)
