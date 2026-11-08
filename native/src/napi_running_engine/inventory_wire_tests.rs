use super::*;
#[test]
fn qualification_and_production_inventory_include_library_membership_and_omit_absence() {
    let mut inventory = crate::engine::export_archive::ImportInventoryDescriptor {
        version: 3,
        relative_filename: ".operation.import-inventory-v3".into(),
        sha256: "a".repeat(64),
        stored_byte_count_hex: "0000000000000400".into(),
        history_count_hex: "0000000000000001".into(),
        hall_of_fame_count_hex: "0000000000000001".into(),
        saved_brains: vec![],
    };
    let empty = serde_json::to_value(import_inventory_to_napi(inventory.clone()).unwrap()).unwrap();
    assert_eq!(
        empty,
        serde_json::json!({"version":3,"relativeFilename":".operation.import-inventory-v3",
        "sha256":"a".repeat(64),"storedByteCount":"0000000000000400",
        "historyCount":"0000000000000001","hallOfFameCount":"0000000000000001"})
    );
    inventory.saved_brains.push(
        crate::engine::saved_brain_archive::SavedBrainArchiveEntryV1 {
            brain_id: "b".repeat(64),
            name: "Manual brain".into(),
            created_at_ms: 42,
            artifact_byte_count: "0000000000001000".into(),
        },
    );
    let actual =
        serde_json::to_value(import_inventory_to_napi(inventory.clone()).unwrap()).unwrap();
    let selected: Vec<crate::engine::saved_brain_archive::SavedBrainArchiveEntryV1> =
        serde_json::from_str(actual["savedBrainsJson"].as_str().unwrap()).unwrap();
    assert_eq!(selected, inventory.saved_brains);
    assert_eq!(actual["historyCount"], empty["historyCount"]);
}
