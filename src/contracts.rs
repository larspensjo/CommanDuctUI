//! Shared platform contracts that are pure enough to be one source of truth.

use crate::{DockStyle, LayoutRule, PlatformError, PlatformResult};
use std::collections::BTreeMap;

pub(crate) fn validate_layout_rules(rules: &[LayoutRule]) -> PlatformResult<()> {
    let mut fill_by_parent: BTreeMap<Option<i32>, Vec<i32>> = BTreeMap::new();
    for rule in rules {
        match rule.dock_style {
            DockStyle::Top | DockStyle::Bottom | DockStyle::Left | DockStyle::Right => {
                if rule.fixed_size.is_none() {
                    return Err(PlatformError::OperationFailed(format!(
                        "DefineLayout rejected: control {} uses {:?} without fixed_size. Docked edges require explicit fixed_size.",
                        rule.control_id.raw(),
                        rule.dock_style
                    )));
                }
                if let Some(size) = rule.fixed_size
                    && size < 0
                {
                    return Err(PlatformError::OperationFailed(format!(
                        "DefineLayout rejected: control {} has negative fixed_size {} for {:?}.",
                        rule.control_id.raw(),
                        size,
                        rule.dock_style
                    )));
                }
            }
            _ => {}
        }

        if rule.dock_style == DockStyle::Fill {
            fill_by_parent
                .entry(rule.parent_control_id.map(|id| id.raw()))
                .or_default()
                .push(rule.control_id.raw());
        }
    }

    for (parent_id, fill_controls) in fill_by_parent {
        if fill_controls.len() > 1 {
            let parent_desc = parent_id
                .map(|id| format!("control {id}"))
                .unwrap_or_else(|| "main window".to_string());
            let control_ids = fill_controls
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(PlatformError::OperationFailed(format!(
                "DefineLayout rejected: parent {parent_desc} has multiple DockStyle::Fill children ({control_ids}). CommanDuctUI supports exactly one Fill child per parent."
            )));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate_layout_rules;
    use crate::{ControlId, DockStyle, LayoutRule, PlatformError};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ContractKind {
        SharedPure,
        ParityOnly,
    }

    #[derive(Debug)]
    struct ContractCatalogEntry {
        id: &'static str,
        kind: ContractKind,
        summary: &'static str,
        pinning: &'static str,
    }

    const CONTRACT_CATALOG: &[ContractCatalogEntry] = &[
        ContractCatalogEntry {
            id: "layout-rule-validation",
            kind: ContractKind::SharedPure,
            summary: "DefineLayout rejects missing/negative fixed sizes for edge docks and more than one Fill child per parent.",
            pinning: "contracts::validate_layout_rules is called by Win32 and headless; tested by validate_layout_rules_contract_table.",
        },
        ContractCatalogEntry {
            id: "programmatic-set-silence",
            kind: ContractKind::ParityOnly,
            summary: "Programmatic Set* commands are event-silent except SetTreeViewSelection.",
            pinning: "Asserted by parity_table_programmatic_set_commands_are_silent_except_tree_selection; Win32 contract is native-message backed.",
        },
        ContractCatalogEntry {
            id: "tree-view-programmatic-selection-event",
            kind: ContractKind::ParityOnly,
            summary: "SetTreeViewSelection updates selection and emits TreeViewItemSelectionChanged.",
            pinning: "Asserted by parity_table_programmatic_set_commands_are_silent_except_tree_selection; Win32 fires TVN_SELCHANGED for TVM_SELECTITEM.",
        },
        ContractCatalogEntry {
            id: "hidden-tree-toggle-suppression",
            kind: ContractKind::ParityOnly,
            summary: "Toggling a CheckState::Hidden tree item leaves it hidden and emits no event.",
            pinning: "Asserted by hidden_tree_toggle_is_silent; Win32 restores the hidden state-image lane.",
        },
        ContractCatalogEntry {
            id: "disabled-listbox-row-selectable",
            kind: ContractKind::ParityOnly,
            summary: "Disabled listbox rows remain selectable.",
            pinning: "Asserted by parity_table_disabled_listbox_rows_remain_user_selectable and documented on ListBoxItemDescriptor::enabled.",
        },
        ContractCatalogEntry {
            id: "radio-group-start-scoping",
            kind: ContractKind::ParityOnly,
            summary: "Radio button grouping is scoped by parent and group_start boundaries.",
            pinning: "Asserted by radio_buttons_are_scoped_by_group_start; current Win32 grouping is implemented through native button groups.",
        },
    ];

    #[derive(Debug)]
    struct LayoutRuleCase {
        name: &'static str,
        rules: Vec<LayoutRule>,
        expected_error_fragments: &'static [&'static str],
    }

    fn rule(control_id: i32, parent: Option<i32>, dock_style: DockStyle) -> LayoutRule {
        LayoutRule {
            control_id: ControlId::new(control_id),
            parent_control_id: parent.map(ControlId::new),
            dock_style,
            order: 0,
            fixed_size: None,
            margin: (0, 0, 0, 0),
        }
    }

    // This catalog is an append-only index by convention: shared-pure entries point at tests
    // in this module, while parity-only entries name their backing headless tests. It is not a
    // compile-time registry of test functions.
    #[test]
    fn contract_catalog_is_append_only_index_for_phase_3a_seed_behaviors() {
        let ids = CONTRACT_CATALOG
            .iter()
            .map(|entry| entry.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "layout-rule-validation",
                "programmatic-set-silence",
                "tree-view-programmatic-selection-event",
                "hidden-tree-toggle-suppression",
                "disabled-listbox-row-selectable",
                "radio-group-start-scoping",
            ]
        );
        assert!(CONTRACT_CATALOG.iter().any(|entry| {
            entry.kind == ContractKind::SharedPure
                && entry.summary.contains("DefineLayout")
                && entry.pinning.contains("validate_layout_rules")
        }));
    }

    #[test]
    fn validate_layout_rules_contract_table() {
        let cases = vec![
            LayoutRuleCase {
                name: "allows one fill per parent",
                rules: vec![
                    rule(10, Some(1), DockStyle::Fill),
                    rule(11, Some(2), DockStyle::Fill),
                    LayoutRule {
                        fixed_size: Some(24),
                        ..rule(12, Some(1), DockStyle::Top)
                    },
                ],
                expected_error_fragments: &[],
            },
            LayoutRuleCase {
                name: "docked edge needs fixed size",
                rules: vec![rule(20, None, DockStyle::Top)],
                expected_error_fragments: &["20", "Top", "fixed_size"],
            },
            LayoutRuleCase {
                name: "negative fixed size is invalid",
                rules: vec![LayoutRule {
                    fixed_size: Some(-1),
                    ..rule(21, None, DockStyle::Left)
                }],
                expected_error_fragments: &["21", "-1", "Left", "fixed_size"],
            },
            LayoutRuleCase {
                name: "one fill child per parent",
                rules: vec![
                    rule(30, Some(7), DockStyle::Fill),
                    rule(31, Some(7), DockStyle::Fill),
                ],
                expected_error_fragments: &["control 7", "Fill", "30", "31"],
            },
        ];

        for case in cases {
            let result = validate_layout_rules(&case.rules);
            if case.expected_error_fragments.is_empty() {
                result.unwrap_or_else(|err| panic!("{} should be valid: {err}", case.name));
                continue;
            }

            let err = result.unwrap_err();
            let PlatformError::OperationFailed(message) = err else {
                panic!("{} returned unexpected error variant: {err:?}", case.name);
            };
            for fragment in case.expected_error_fragments {
                assert!(
                    message.contains(fragment),
                    "{} error should contain {fragment:?}: {message}",
                    case.name
                );
            }
        }
    }
}
