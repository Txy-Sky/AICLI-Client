#!/usr/bin/env python3
"""Parse every workflow/composite action and reject duplicate YAML keys."""

from pathlib import Path

import yaml


class UniqueKeyLoader(yaml.SafeLoader):
    pass


def unique_mapping(loader, node):
    loader.flatten_mapping(node)
    result = {}
    for key_node, value_node in node.value:
        key = loader.construct_object(key_node)
        if key in result:
            raise yaml.constructor.ConstructorError(
                "mapping", node.start_mark, f"duplicate key: {key}", key_node.start_mark
            )
        result[key] = loader.construct_object(value_node)
    return result


UniqueKeyLoader.add_constructor(
    yaml.resolver.BaseResolver.DEFAULT_MAPPING_TAG, unique_mapping
)

root = Path(__file__).resolve().parents[1]
paths = sorted(
    path
    for folder in (root / ".github/workflows", root / ".github/actions")
    for path in folder.rglob("*")
    if path.is_file() and path.suffix in (".yml", ".yaml")
)
if not paths:
    raise SystemExit("No workflow YAML files found")
for path in paths:
    with path.open() as stream:
        document = yaml.load(stream, Loader=UniqueKeyLoader)
    if not isinstance(document, dict):
        raise SystemExit(f"Expected a YAML mapping: {path.relative_to(root)}")
print(f"Workflow YAML syntax: {len(paths)} files passed; no duplicate keys")
