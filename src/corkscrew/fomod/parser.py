"""FOMOD installer XML parser.

Parses ModuleConfig.xml files that define mod installation options.
FOMOD is a standard format used by Nexus Mods mod managers.

Reference: https://fomod-docs.readthedocs.io/
"""

import xml.etree.ElementTree as ET
from dataclasses import dataclass, field
from pathlib import Path


@dataclass
class FomodFile:
    """A file/folder to install."""

    source: str
    destination: str
    priority: int = 0
    is_folder: bool = False


@dataclass
class FomodOption:
    """A selectable option within an installation group."""

    name: str
    description: str
    image: str | None
    files: list[FomodFile] = field(default_factory=list)
    type_descriptor: str = "Optional"  # Required, Recommended, Optional, NotUsable, CouldBeUsable


@dataclass
class FomodGroup:
    """A group of options (radio buttons or checkboxes)."""

    name: str
    group_type: str  # SelectExactlyOne, SelectAtMostOne, SelectAtLeastOne, SelectAll, SelectAny
    options: list[FomodOption] = field(default_factory=list)


@dataclass
class FomodStep:
    """An installation step (page in the wizard)."""

    name: str
    groups: list[FomodGroup] = field(default_factory=list)
    visibility_conditions: list = field(default_factory=list)


@dataclass
class FomodInstaller:
    """Parsed FOMOD installer configuration."""

    module_name: str
    required_files: list[FomodFile] = field(default_factory=list)
    steps: list[FomodStep] = field(default_factory=list)
    conditional_installs: list = field(default_factory=list)


def parse_fomod(fomod_dir: Path) -> FomodInstaller | None:
    """Parse a FOMOD installer from a directory containing ModuleConfig.xml.

    Returns None if no FOMOD config is found.
    """
    # FOMOD config can be in fomod/ or root
    config_path = None
    for candidate in [
        fomod_dir / "fomod" / "ModuleConfig.xml",
        fomod_dir / "fomod" / "moduleconfig.xml",
    ]:
        if candidate.exists():
            config_path = candidate
            break

    if not config_path:
        return None

    tree = ET.parse(config_path)
    root = tree.getroot()

    # Strip namespace if present
    ns = ""
    if root.tag.startswith("{"):
        ns = root.tag.split("}")[0] + "}"

    def find(element, tag):
        return element.find(f"{ns}{tag}")

    def findall(element, tag):
        return element.findall(f"{ns}{tag}")

    installer = FomodInstaller(
        module_name=find(root, "moduleName").text if find(root, "moduleName") is not None else "Unknown",
    )

    # Parse required files
    required = find(root, "requiredInstallFiles")
    if required is not None:
        for file_elem in list(required):
            installer.required_files.append(_parse_file(file_elem, ns))

    # Parse installation steps
    steps_elem = find(root, "installSteps")
    if steps_elem is not None:
        for step_elem in findall(steps_elem, "installStep"):
            step = FomodStep(name=step_elem.get("name", ""))

            groups_elem = find(step_elem, "optionalFileGroups")
            if groups_elem is not None:
                for group_elem in findall(groups_elem, "group"):
                    group = FomodGroup(
                        name=group_elem.get("name", ""),
                        group_type=group_elem.get("type", "SelectAny"),
                    )

                    plugins_elem = find(group_elem, "plugins")
                    if plugins_elem is not None:
                        for plugin_elem in findall(plugins_elem, "plugin"):
                            option = _parse_option(plugin_elem, ns)
                            group.options.append(option)

                    step.groups.append(group)

            installer.steps.append(step)

    return installer


def _parse_file(elem, ns: str) -> FomodFile:
    """Parse a file or folder element."""
    tag = elem.tag.replace(ns, "")
    return FomodFile(
        source=elem.get("source", ""),
        destination=elem.get("destination", ""),
        priority=int(elem.get("priority", "0")),
        is_folder=(tag == "folder"),
    )


def _parse_option(plugin_elem, ns: str) -> FomodOption:
    """Parse a plugin (option) element."""
    def find(element, tag):
        return element.find(f"{ns}{tag}")

    desc_elem = find(plugin_elem, "description")
    image_elem = find(plugin_elem, "image")
    type_elem = find(plugin_elem, "typeDescriptor")

    option = FomodOption(
        name=plugin_elem.get("name", ""),
        description=desc_elem.text if desc_elem is not None else "",
        image=image_elem.get("path") if image_elem is not None else None,
    )

    # Parse type descriptor
    if type_elem is not None:
        default_type = find(type_elem, "type")
        if default_type is not None:
            option.type_descriptor = default_type.get("name", "Optional")

    # Parse files
    files_elem = find(plugin_elem, "files")
    if files_elem is not None:
        for file_elem in list(files_elem):
            option.files.append(_parse_file(file_elem, ns))

    return option


def get_required_files(installer: FomodInstaller) -> list[FomodFile]:
    """Get files that must always be installed."""
    return installer.required_files


def get_default_selections(installer: FomodInstaller) -> dict[str, list[str]]:
    """Get default selections for each group based on type descriptors.

    Returns {group_name: [selected_option_names]}.
    """
    selections: dict[str, list[str]] = {}

    for step in installer.steps:
        for group in step.groups:
            selected: list[str] = []

            if group.group_type == "SelectAll":
                selected = [opt.name for opt in group.options]
            else:
                for opt in group.options:
                    if opt.type_descriptor in ("Required", "Recommended"):
                        selected.append(opt.name)

                # For SelectExactlyOne, ensure exactly one is selected
                if group.group_type == "SelectExactlyOne" and len(selected) != 1:
                    if group.options:
                        selected = [group.options[0].name]

            selections[group.name] = selected

    return selections


def get_files_for_selections(
    installer: FomodInstaller,
    selections: dict[str, list[str]],
) -> list[FomodFile]:
    """Resolve which files to install based on user selections."""
    files: list[FomodFile] = list(installer.required_files)

    for step in installer.steps:
        for group in step.groups:
            selected_names = set(selections.get(group.name, []))
            for option in group.options:
                if option.name in selected_names:
                    files.extend(option.files)

    # Sort by priority (higher priority = installed later = wins conflicts)
    files.sort(key=lambda f: f.priority)
    return files
