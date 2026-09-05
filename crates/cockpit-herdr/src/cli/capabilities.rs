use std::collections::BTreeSet;

pub(crate) const REQUIRED_METHODS: [&str; 24] = [
    "ping",
    "session.snapshot",
    "events.subscribe",
    "worktree.list",
    "pane.read",
    "workspace.focus",
    "tab.focus",
    "pane.focus",
    "agent.focus",
    "workspace.create",
    "workspace.rename",
    "workspace.move_block",
    "workspace.close",
    "tab.create",
    "tab.rename",
    "tab.move",
    "tab.close",
    "pane.split",
    "pane.resize",
    "pane.rename",
    "pane.swap",
    "pane.move",
    "pane.zoom",
    "pane.close",
];

pub(crate) fn missing_required_methods(methods: &BTreeSet<String>) -> Vec<&'static str> {
    REQUIRED_METHODS
        .iter()
        .copied()
        .filter(|method| !methods.contains(*method))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{REQUIRED_METHODS, missing_required_methods};
    use std::collections::BTreeSet;

    #[test]
    fn reports_missing_focus_methods() {
        let methods = REQUIRED_METHODS
            .iter()
            .filter(|method| **method != "agent.focus")
            .map(|method| (*method).to_owned())
            .collect::<BTreeSet<_>>();
        assert_eq!(missing_required_methods(&methods), vec!["agent.focus"]);
    }
}
