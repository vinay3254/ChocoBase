use std::fs;
use std::path::Path;

#[test]
fn test_sdk_package_structure_and_ci_workflow() {
    let manifest_path = Path::new("sdk/typescript/package.json");
    assert!(manifest_path.exists(), "SDK package.json missing");
    let manifest_str = fs::read_to_string(manifest_path).unwrap();
    let json: serde_json::Value = serde_json::from_str(&manifest_str).unwrap();
    assert_eq!(json["name"], "@chocobase/chocobase-js");
    assert_eq!(json["version"], "0.1.0");

    let index_path = Path::new("sdk/typescript/src/index.ts");
    assert!(index_path.exists(), "SDK index.ts missing");
    let index_src = fs::read_to_string(index_path).unwrap();
    assert!(index_src.contains("export class ChocoBaseClient"));
    assert!(index_src.contains("export function createClient"));

    let ci_path = Path::new(".github/workflows/ci.yml");
    assert!(ci_path.exists(), ".github/workflows/ci.yml missing");
    let ci_str = fs::read_to_string(ci_path).unwrap();
    assert!(ci_str.contains("cargo test"));
    assert!(ci_str.contains("cargo clippy"));
    assert!(ci_str.contains("cargo fmt"));
}
