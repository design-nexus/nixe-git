use gtk::prelude::*;

pub fn policy() -> gtk::PolicyType {
    if crate::prefs::get().show_scrollbars { gtk::PolicyType::Automatic } else { gtk::PolicyType::External }
}
pub fn apply() {
    fn walk(widget: &gtk::Widget, policy: gtk::PolicyType, visible: bool) {
        if let Some(scroll) = widget.downcast_ref::<gtk::ScrolledWindow>() {
            if scroll.hscrollbar_policy() != gtk::PolicyType::Never {
                scroll.set_hscrollbar_policy(policy);
            }
            if scroll.vscrollbar_policy() != gtk::PolicyType::Never {
                scroll.set_vscrollbar_policy(policy);
            }
        }
        if widget.has_css_class("git-graph-scrollbar") {
            widget.set_visible(visible);
        }
        let mut child = widget.first_child();
        while let Some(next) = child {
            child = next.next_sibling();
            walk(&next, policy, visible);
        }
    }
    let visible = crate::prefs::get().show_scrollbars;
    let policy = policy();
    for window in gtk::Window::list_toplevels() {
        walk(&window, policy, visible);
    }
}
