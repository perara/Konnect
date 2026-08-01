use konnect_core::router::{meta_tools, registry};
use std::collections::BTreeSet;

#[test]
fn generated_tool_directory_matches_the_runtime_registry() {
    let directory = include_str!("../../../tool-directory.md");
    let documented: Vec<&str> = directory
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|line| line.split_once('`').map(|(name, _)| name))
        .collect();
    let unique: BTreeSet<&str> = documented.iter().copied().collect();
    assert_eq!(
        unique.len(),
        documented.len(),
        "tool-directory.md contains duplicate tool rows"
    );

    let registered: Vec<&str> = registry::ALL_TOOLSETS
        .iter()
        .flat_map(|toolset| registry::tools_for(toolset.name).unwrap())
        .map(|tool| tool.name)
        .collect();
    let meta = meta_tools::meta_tool_descriptions();
    assert_eq!(
        documented.len(),
        registered.len() + meta.len(),
        "tool-directory.md total does not match registered plus meta-tools"
    );
    for name in registered
        .into_iter()
        .chain(meta.iter().map(|tool| tool.name.as_str()))
    {
        assert!(
            unique.contains(name),
            "runtime tool '{name}' is missing from tool-directory.md"
        );
    }

    let registered_count: usize = registry::ALL_TOOLSETS
        .iter()
        .map(|toolset| toolset.tool_count)
        .sum();
    let overview = format!(
        "**{registered_count} registered tools** + **{} always-visible meta-tools** = **{} total**",
        meta.len(),
        registered_count + meta.len()
    );
    assert!(
        directory.contains(&overview),
        "tool-directory.md overview is stale; expected '{overview}'"
    );
}

#[test]
fn maintenance_tool_schemas_match_their_public_contracts() {
    let cases = [
        (
            "sch_components",
            "sync_embedded_symbol_from_library",
            ["lib_id", "library_path", "schematic"].as_slice(),
            ["lib_id", "library_path", "schematic"].as_slice(),
        ),
        (
            "sch_wiring",
            "normalize_schematic_junctions",
            ["schematic"].as_slice(),
            ["schematic"].as_slice(),
        ),
        (
            "library",
            "normalize_symbol_library",
            ["apply", "library_path"].as_slice(),
            ["library_path"].as_slice(),
        ),
    ];

    for (toolset, name, expected_properties, expected_required) in cases {
        let tool = registry::tools_for(toolset)
            .unwrap()
            .into_iter()
            .find(|tool| tool.name == name)
            .unwrap_or_else(|| panic!("missing maintenance tool '{name}'"));
        let properties: BTreeSet<&str> = tool.input_schema["properties"]
            .as_object()
            .expect("properties object")
            .keys()
            .map(String::as_str)
            .collect();
        let required: BTreeSet<&str> = tool.input_schema["required"]
            .as_array()
            .expect("required array")
            .iter()
            .map(|value| value.as_str().expect("required name"))
            .collect();
        assert_eq!(properties, expected_properties.iter().copied().collect());
        assert_eq!(required, expected_required.iter().copied().collect());
    }

    let library_tool = registry::tools_for("library")
        .unwrap()
        .into_iter()
        .find(|tool| tool.name == "normalize_symbol_library")
        .unwrap();
    assert_eq!(
        library_tool.input_schema["properties"]["apply"]["default"],
        false
    );
}
