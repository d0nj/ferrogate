use sea_orm::{entity::prelude::StringLen, DeriveActiveEnum, EnumIter};

#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    EnumIter,
    serde::Serialize,
    serde::Deserialize,
    DeriveActiveEnum,
)]
#[serde(rename_all = "snake_case")]
#[sea_orm(rs_type = "String", db_type = "String(StringLen::N(16))")]
pub enum Role {
    #[sea_orm(string_value = "owner")]
    Owner,
    #[sea_orm(string_value = "admin")]
    Admin,
    #[sea_orm(string_value = "operator")]
    Operator,
    #[sea_orm(string_value = "viewer")]
    Viewer,
}

impl Role {
    pub const ALL: [Role; 4] = [Role::Owner, Role::Admin, Role::Operator, Role::Viewer];

    pub fn as_str(self) -> &'static str {
        match self {
            Role::Owner => "owner",
            Role::Admin => "admin",
            Role::Operator => "operator",
            Role::Viewer => "viewer",
        }
    }

    pub fn from_str_strict(raw: &str) -> Option<Role> {
        match raw {
            "owner" => Some(Role::Owner),
            "admin" => Some(Role::Admin),
            "operator" => Some(Role::Operator),
            "viewer" => Some(Role::Viewer),
            _ => None,
        }
    }

    pub fn rank(self) -> u8 {
        match self {
            Role::Owner => 3,
            Role::Admin => 2,
            Role::Operator => 1,
            Role::Viewer => 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Capability {
    ReadOverview,
    ReadLogs,
    ReadConfig,
    ManageProviders,
    ManageRouting,
    ManageKeys,
    ManageGuardrails,
    ManageWebhooks,
    ManagePrompts,
    ManageMcp,
    ManagePricing,
    ManageUsers,
    ClearCache,
    ClearLogs,
    ChangeSettings,
}

impl Capability {
    pub fn as_str(self) -> &'static str {
        match self {
            Capability::ReadOverview => "read_overview",
            Capability::ReadLogs => "read_logs",
            Capability::ReadConfig => "read_config",
            Capability::ManageProviders => "manage_providers",
            Capability::ManageRouting => "manage_routing",
            Capability::ManageKeys => "manage_keys",
            Capability::ManageGuardrails => "manage_guardrails",
            Capability::ManageWebhooks => "manage_webhooks",
            Capability::ManagePrompts => "manage_prompts",
            Capability::ManageMcp => "manage_mcp",
            Capability::ManagePricing => "manage_pricing",
            Capability::ManageUsers => "manage_users",
            Capability::ClearCache => "clear_cache",
            Capability::ClearLogs => "clear_logs",
            Capability::ChangeSettings => "change_settings",
        }
    }
}

const VIEWER_CAPABILITIES: &[Capability] = &[
    Capability::ReadOverview,
    Capability::ReadLogs,
    Capability::ReadConfig,
];

const OPERATOR_CAPABILITIES: &[Capability] = &[
    Capability::ReadOverview,
    Capability::ReadLogs,
    Capability::ReadConfig,
    Capability::ManageProviders,
    Capability::ManageRouting,
    Capability::ManageKeys,
    Capability::ManageGuardrails,
    Capability::ManageWebhooks,
    Capability::ManagePrompts,
    Capability::ManageMcp,
    Capability::ClearCache,
    Capability::ClearLogs,
];

const ADMIN_CAPABILITIES: &[Capability] = &[
    Capability::ReadOverview,
    Capability::ReadLogs,
    Capability::ReadConfig,
    Capability::ManageProviders,
    Capability::ManageRouting,
    Capability::ManageKeys,
    Capability::ManageGuardrails,
    Capability::ManageWebhooks,
    Capability::ManagePrompts,
    Capability::ManageMcp,
    Capability::ClearCache,
    Capability::ClearLogs,
    Capability::ManagePricing,
    Capability::ChangeSettings,
];

const OWNER_CAPABILITIES: &[Capability] = &[
    Capability::ReadOverview,
    Capability::ReadLogs,
    Capability::ReadConfig,
    Capability::ManageProviders,
    Capability::ManageRouting,
    Capability::ManageKeys,
    Capability::ManageGuardrails,
    Capability::ManageWebhooks,
    Capability::ManagePrompts,
    Capability::ManageMcp,
    Capability::ClearCache,
    Capability::ClearLogs,
    Capability::ManagePricing,
    Capability::ChangeSettings,
    Capability::ManageUsers,
];

pub fn capabilities_for(role: Role) -> &'static [Capability] {
    match role {
        Role::Owner => OWNER_CAPABILITIES,
        Role::Admin => ADMIN_CAPABILITIES,
        Role::Operator => OPERATOR_CAPABILITIES,
        Role::Viewer => VIEWER_CAPABILITIES,
    }
}

pub fn grants(role: Role, capability: Capability) -> bool {
    capabilities_for(role).contains(&capability)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rank_is_strictly_ordered() {
        assert_eq!(Role::Owner.rank(), 3);
        assert_eq!(Role::Admin.rank(), 2);
        assert_eq!(Role::Operator.rank(), 1);
        assert_eq!(Role::Viewer.rank(), 0);
        assert!(Role::Owner.rank() > Role::Admin.rank());
        assert!(Role::Admin.rank() > Role::Operator.rank());
        assert!(Role::Operator.rank() > Role::Viewer.rank());
    }

    #[test]
    fn from_str_strict_round_trips() {
        for role in Role::ALL {
            assert_eq!(Role::from_str_strict(role.as_str()), Some(role));
        }
        assert_eq!(Role::from_str_strict("Owner"), None);
        assert_eq!(Role::from_str_strict("superuser"), None);
        assert_eq!(Role::from_str_strict(""), None);
    }

    #[test]
    fn grants_matrix_is_pinned_cell_by_cell() {
        use Capability::*;
        use Role::*;

        #[rustfmt::skip]
        let expected: [(Role, Capability, bool); 60] = [
            (Viewer,   ReadOverview, true),
            (Viewer,   ReadLogs, true),
            (Viewer,   ReadConfig, true),
            (Viewer,   ManageProviders, false),
            (Viewer,   ManageRouting, false),
            (Viewer,   ManageKeys, false),
            (Viewer,   ManageGuardrails, false),
            (Viewer,   ManageWebhooks, false),
            (Viewer,   ManagePrompts, false),
            (Viewer,   ManageMcp, false),
            (Viewer,   ManagePricing, false),
            (Viewer,   ManageUsers, false),
            (Viewer,   ClearCache, false),
            (Viewer,   ClearLogs, false),
            (Viewer,   ChangeSettings, false),

            (Operator, ReadOverview, true),
            (Operator, ReadLogs, true),
            (Operator, ReadConfig, true),
            (Operator, ManageProviders, true),
            (Operator, ManageRouting, true),
            (Operator, ManageKeys, true),
            (Operator, ManageGuardrails, true),
            (Operator, ManageWebhooks, true),
            (Operator, ManagePrompts, true),
            (Operator, ManageMcp, true),
            (Operator, ManagePricing, false),
            (Operator, ManageUsers, false),
            (Operator, ClearCache, true),
            (Operator, ClearLogs, true),
            (Operator, ChangeSettings, false),

            (Admin,    ReadOverview, true),
            (Admin,    ReadLogs, true),
            (Admin,    ReadConfig, true),
            (Admin,    ManageProviders, true),
            (Admin,    ManageRouting, true),
            (Admin,    ManageKeys, true),
            (Admin,    ManageGuardrails, true),
            (Admin,    ManageWebhooks, true),
            (Admin,    ManagePrompts, true),
            (Admin,    ManageMcp, true),
            (Admin,    ManagePricing, true),
            (Admin,    ManageUsers, false),
            (Admin,    ClearCache, true),
            (Admin,    ClearLogs, true),
            (Admin,    ChangeSettings, true),

            (Owner,    ReadOverview, true),
            (Owner,    ReadLogs, true),
            (Owner,    ReadConfig, true),
            (Owner,    ManageProviders, true),
            (Owner,    ManageRouting, true),
            (Owner,    ManageKeys, true),
            (Owner,    ManageGuardrails, true),
            (Owner,    ManageWebhooks, true),
            (Owner,    ManagePrompts, true),
            (Owner,    ManageMcp, true),
            (Owner,    ManagePricing, true),
            (Owner,    ManageUsers, true),
            (Owner,    ClearCache, true),
            (Owner,    ClearLogs, true),
            (Owner,    ChangeSettings, true),
        ];

        assert_eq!(expected.len(), 60);
        for (role, capability, allowed) in expected {
            assert_eq!(
                grants(role, capability),
                allowed,
                "{} / {}",
                role.as_str(),
                capability.as_str()
            );
        }
    }

    #[test]
    fn only_owner_manages_users() {
        let granting: Vec<&'static str> = Role::ALL
            .into_iter()
            .filter(|role| grants(*role, Capability::ManageUsers))
            .map(Role::as_str)
            .collect();
        assert_eq!(granting, vec!["owner"]);
    }

    #[test]
    fn higher_roles_are_supersets_of_lower_roles() {
        for pair in Role::ALL.windows(2) {
            let (higher, lower) = (pair[0], pair[1]);
            for capability in capabilities_for(lower) {
                assert!(
                    grants(higher, *capability),
                    "{} must grant everything {} does",
                    higher.as_str(),
                    lower.as_str()
                );
            }
        }
    }

    #[test]
    fn capability_labels_are_unique() {
        let labels: Vec<&'static str> = capabilities_for(Role::Owner)
            .iter()
            .map(|capability| capability.as_str())
            .collect();
        let mut sorted = labels.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(sorted.len(), labels.len());
    }
}
