//! Integration tests for Control Plane Billing Tiers and Quota Adjustment Engine.

use dbengine::control_plane::{BillingTier, ControlPlane};

#[test]
fn test_billing_tier_quota_scaling() {
    let cp = ControlPlane::global();
    let org = cp.create_organization("Acme Corp");
    let project = cp.create_project(&org.id, "Production App", "us-east-1").unwrap();

    // 1. Initial default tier is Free
    assert_eq!(project.tier, BillingTier::Free);
    assert_eq!(project.quota.max_storage_mb, 500);
    assert_eq!(project.quota.max_egress_mb, 2048);
    assert_eq!(project.quota.max_realtime_connections, 200);
    assert_eq!(project.quota.max_functions, 10);

    // 2. Upgrade to Pro Tier
    let upgraded_pro = cp.update_project_tier(&project.id, BillingTier::Pro).unwrap();
    assert_eq!(upgraded_pro.tier, BillingTier::Pro);
    assert_eq!(upgraded_pro.quota.max_storage_mb, 8192);
    assert_eq!(upgraded_pro.quota.max_egress_mb, 51200);
    assert_eq!(upgraded_pro.quota.max_realtime_connections, 5000);
    assert_eq!(upgraded_pro.quota.max_functions, 100);

    // 3. Upgrade to Enterprise Tier
    let upgraded_ent = cp.update_project_tier(&project.id, BillingTier::Enterprise).unwrap();
    assert_eq!(upgraded_ent.tier, BillingTier::Enterprise);
    assert_eq!(upgraded_ent.quota.max_storage_mb, 102400);
    assert_eq!(upgraded_ent.quota.max_egress_mb, 1048576);
    assert_eq!(upgraded_ent.quota.max_realtime_connections, 50000);
    assert_eq!(upgraded_ent.quota.max_functions, 1000);

    // 4. Resolve project via anon_key
    let resolved = cp.resolve_project(&project.anon_key);
    assert!(resolved.is_some());
    assert_eq!(resolved.unwrap().id, project.id);
}
