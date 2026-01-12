#!/usr/bin/env python3
"""
Extract JSON Schema definitions from GoToSocial's swagger.yaml

This script parses the OpenAPI/Swagger YAML file and extracts
schema definitions for Mastodon API objects, converting them
to JSON Schema format for use in validation tests.

Usage:
    python3 extract_schemas.py [--output-dir tests/schemas]
"""

import yaml
import json
import sys
import argparse
from pathlib import Path
from typing import Dict, Any


def convert_openapi_to_jsonschema(openapi_schema: Dict[str, Any]) -> Dict[str, Any]:
    """
    Convert OpenAPI schema definition to JSON Schema format.
    
    Args:
        openapi_schema: OpenAPI schema object
        
    Returns:
        JSON Schema object
    """
    json_schema = {
        "$schema": "http://json-schema.org/draft-07/schema#",
        "type": openapi_schema.get("type", "object")
    }
    
    # Copy title and description
    if "title" in openapi_schema:
        json_schema["title"] = openapi_schema["title"]
    if "description" in openapi_schema:
        json_schema["description"] = openapi_schema["description"]
    
    # Copy properties
    if "properties" in openapi_schema:
        json_schema["properties"] = {}
        for prop_name, prop_def in openapi_schema["properties"].items():
            json_schema["properties"][prop_name] = convert_property(prop_def)
    
    # Copy required fields
    if "required" in openapi_schema:
        json_schema["required"] = openapi_schema["required"]
    
    return json_schema


def convert_property(prop: Dict[str, Any]) -> Dict[str, Any]:
    """Convert a single property definition."""
    result = {}
    
    # Handle $ref
    if "$ref" in prop:
        # For now, just note it as an object
        # In a full implementation, we'd resolve the reference
        return {"type": "object", "description": f"Reference: {prop['$ref']}"}
    
    # Copy type
    if "type" in prop:
        result["type"] = prop["type"]
    
    # Copy description
    if "description" in prop:
        result["description"] = prop["description"]
    
    # Copy example
    if "example" in prop:
        result["example"] = prop["example"]
    
    # Handle arrays
    if prop.get("type") == "array" and "items" in prop:
        result["items"] = convert_property(prop["items"])
    
    # Handle format
    if "format" in prop:
        result["format"] = prop["format"]
    
    # Handle enum
    if "enum" in prop:
        result["enum"] = prop["enum"]
    
    return result


def extract_schemas(swagger_path: Path, output_dir: Path, schema_names: list = None):
    """
    Extract schemas from swagger.yaml and save as JSON Schema files.
    
    Args:
        swagger_path: Path to swagger.yaml
        output_dir: Directory to save JSON Schema files
        schema_names: List of schema names to extract (None = all)
    """
    # Load swagger.yaml
    with open(swagger_path, 'r') as f:
        swagger = yaml.safe_load(f)
    
    definitions = swagger.get('definitions', {})
    
    # Default schemas to extract if not specified
    if schema_names is None:
        schema_names = [
            'account',
            'status',
            'instance',
            'accountRelationship',
            'poll',
            'notification',
            'list',
            'filter',
            'filterV2',
            'conversation',
            'scheduledStatus',
            'mediaAttachment',
            'emoji',
            'tag',
            'card',
            'application'
        ]
    
    # Create output directory
    output_dir.mkdir(parents=True, exist_ok=True)
    
    # Extract each schema
    extracted = 0
    for schema_name in schema_names:
        if schema_name in definitions:
            schema_def = definitions[schema_name]
            json_schema = convert_openapi_to_jsonschema(schema_def)
            
            # Determine output filename
            # Convert camelCase to snake_case
            output_name = ''.join(['_' + c.lower() if c.isupper() else c for c in schema_name]).lstrip('_')
            output_file = output_dir / f"{output_name}.json"
            
            # Save JSON Schema
            with open(output_file, 'w') as f:
                json.dump(json_schema, f, indent=2)
            
            print(f"✓ Extracted {schema_name} -> {output_file}")
            extracted += 1
        else:
            print(f"✗ Schema '{schema_name}' not found in swagger.yaml", file=sys.stderr)
    
    print(f"\nExtracted {extracted} schemas to {output_dir}")


def main():
    parser = argparse.ArgumentParser(description='Extract JSON Schemas from GoToSocial swagger.yaml')
    parser.add_argument(
        '--swagger',
        type=Path,
        default=Path('gotosocial/docs/api/swagger.yaml'),
        help='Path to swagger.yaml file'
    )
    parser.add_argument(
        '--output-dir',
        type=Path,
        default=Path('tests/schemas'),
        help='Output directory for JSON Schema files'
    )
    parser.add_argument(
        '--schemas',
        nargs='+',
        help='Specific schema names to extract (default: all common schemas)'
    )
    
    args = parser.parse_args()
    
    if not args.swagger.exists():
        print(f"Error: swagger.yaml not found at {args.swagger}", file=sys.stderr)
        sys.exit(1)
    
    extract_schemas(args.swagger, args.output_dir, args.schemas)


if __name__ == '__main__':
    main()
