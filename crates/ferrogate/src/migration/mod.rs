pub mod m20260101_000001_init;
pub mod m20260101_000002_seed;
pub mod m20260301_000003_control_plane_extensions;
pub mod m20260304_000004_rbac;
pub mod m20260401_000005_sessions;
pub mod m20260402_000006_providers_meta_nvidia;
pub mod m20260403_000007_provider_api_dialect;
pub mod m20260403_000008_seal_credentials;
pub mod m20260403_000009_clear_seeded_models;
pub mod m20260403_000010_clear_seeded_routing;
pub mod m20260404_000011_routing_policy;
pub mod m20260405_000012_route_positions;
pub mod m20260406_000013_hardening;

use sea_orm_migration::prelude::*;

pub struct Migrator;

#[async_trait::async_trait]
impl MigratorTrait for Migrator {
    fn migrations() -> Vec<Box<dyn MigrationTrait>> {
        vec![
            Box::new(m20260101_000001_init::Migration),
            Box::new(m20260101_000002_seed::Migration),
            Box::new(m20260301_000003_control_plane_extensions::Migration),
            Box::new(m20260304_000004_rbac::Migration),
            Box::new(m20260401_000005_sessions::Migration),
            Box::new(m20260402_000006_providers_meta_nvidia::Migration),
            Box::new(m20260403_000007_provider_api_dialect::Migration),
            Box::new(m20260403_000008_seal_credentials::Migration),
            Box::new(m20260403_000009_clear_seeded_models::Migration),
            Box::new(m20260403_000010_clear_seeded_routing::Migration),
            Box::new(m20260404_000011_routing_policy::Migration),
            Box::new(m20260405_000012_route_positions::Migration),
            Box::new(m20260406_000013_hardening::Migration),
        ]
    }
}
