//! Which optional mods of a pack get installed. This is the Java edition's "Select Mods To Install" window with the
//! window taken out: the rules (linked mods, groups where only one can be on, dependencies, recommended mods,
//! what a reinstall remembers) live here, and a screen only draws the boxes and calls [`ModChoices::toggle`].

use std::collections::HashMap;

use super::json::PackMod;

/// What an earlier install of the same instance remembers about each mod, by lower-case name.
#[derive(Default, Clone, Debug)]
pub struct Remembered {
    /// (was installed, was selected)
    pub by_name: HashMap<String, (bool, bool)>,
}

impl Remembered {
    pub fn from_records(records: &[crate::modrecord::ModRecord]) -> Remembered {
        let mut by_name = HashMap::new();
        for r in records {
            // the first record with a name is the one asked about, the same as the Java edition
            by_name.entry(r.name.to_lowercase()).or_insert((true, r.was_selected));
        }
        Remembered { by_name }
    }

    fn was_installed(&self, name: &str) -> bool {
        self.by_name.contains_key(&name.to_lowercase())
    }

    fn was_selected(&self, name: &str) -> bool {
        self.by_name.get(&name.to_lowercase()).is_some_and(|(_, s)| *s)
    }
}

/// One mod of the pack with its box.
#[derive(Clone, Debug)]
pub struct Choice {
    pub pack_mod: PackMod,
    pub selected: bool,
    /// A box that is greyed out: required mods, and linked mods whose parent is off.
    pub enabled: bool,
    /// Whether this mod is optional for the kind of install being done.
    pub optional: bool,
    /// Hidden mods and libraries are installed but not listed.
    pub visible: bool,
}

#[derive(Clone, Debug)]
pub struct ModChoices {
    /// In install order (`sortMods`): required first, linked mods right after their parent.
    pub choices: Vec<Choice>,
    pub is_server: bool,
    sorted_out: Vec<usize>,
}

/// Required mods first, then the optional ones, with every linked mod placed right behind the mod it is linked to.
pub fn sort_mods(original: &[PackMod]) -> Vec<PackMod> {
    let mut mods: Vec<PackMod> = original.to_vec();
    for m in original {
        if m.optional && m.has_linked() {
            for parent in original {
                if parent.name.eq_ignore_ascii_case(m.linked.as_deref().unwrap_or("")) {
                    if let Some(pos) = mods.iter().position(|x| x.name == m.name) {
                        let moved = mods.remove(pos);
                        let at = mods.iter().position(|x| x.name == parent.name).map(|i| i + 1).unwrap_or(mods.len());
                        mods.insert(at.min(mods.len()), moved);
                    }
                }
            }
        }
    }
    let mut out: Vec<PackMod> = mods.iter().filter(|m| !m.optional).cloned().collect();
    for m in &mods {
        if !out.iter().any(|o| o.name == m.name && o.file == m.file && o.url == m.url) {
            out.push(m.clone());
        }
    }
    out
}

impl ModChoices {
    /// Set the boxes up the way the Java window does when it opens.
    pub fn new(all: Vec<PackMod>, is_server: bool, remembered: Option<&Remembered>) -> anyhow::Result<ModChoices> {
        let sorted = sort_mods(&all);
        let mut c = ModChoices { choices: Vec::new(), is_server, sorted_out: Vec::new() };
        for m in sorted {
            let optional = m.is_optional(is_server);
            c.choices.push(Choice {
                visible: !(m.hidden || m.library),
                selected: !optional,
                enabled: false,
                optional,
                pack_mod: m,
            });
        }

        // boxes are created in alphabetical order, and a linked mod looks at the boxes made before it
        let mut order: Vec<usize> = (0..c.choices.len()).collect();
        order.sort_by(|a, b| {
            c.choices[*a].pack_mod.name.to_lowercase().cmp(&c.choices[*b].pack_mod.name.to_lowercase())
        });
        let mut created: Vec<usize> = Vec::new();
        for &i in &order {
            let m = c.choices[i].pack_mod.clone();
            if c.choices[i].optional {
                c.choices[i].enabled = true;
                c.choices[i].selected = false;
                if m.has_linked() {
                    let parent_name = m.linked.clone().unwrap_or_default();
                    let Some(parent) = c.index_of(&parent_name) else {
                        anyhow::bail!(
                            "The mod {} tried to reference a linked mod {} which doesn't exist!",
                            m.name,
                            parent_name
                        );
                    };
                    if c.choices[parent].optional {
                        c.choices[i].enabled = false;
                    }
                    if m.selected {
                        c.choices[i].enabled = true;
                        c.choices[i].selected = true;
                        if !c.choices[parent].pack_mod.selected {
                            let mut need_children = false;
                            for &j in &created {
                                if c.choices[j].pack_mod.name.eq_ignore_ascii_case(&parent_name) {
                                    c.choices[j].selected = true;
                                    need_children = true;
                                    break;
                                }
                            }
                            if need_children {
                                for &j in &created {
                                    if c.choices[j]
                                        .pack_mod
                                        .linked
                                        .as_deref()
                                        .is_some_and(|l| l.eq_ignore_ascii_case(&parent_name))
                                    {
                                        c.choices[j].enabled = true;
                                    }
                                }
                            }
                        }
                    } else if c.choices[parent].pack_mod.selected {
                        c.choices[i].enabled = true;
                    }
                }
            } else {
                c.choices[i].selected = true;
                c.choices[i].enabled = false;
            }

            if let Some(r) = remembered {
                if !r.was_selected(&m.name) {
                    if c.choices[i].optional {
                        c.choices[i].selected = false;
                        c.choices[i].enabled = true;
                    }
                } else if r.was_installed(&m.name) && c.choices[i].optional {
                    c.choices[i].selected = true;
                    c.choices[i].enabled = true;
                }
            } else if c.choices[i].optional && m.selected {
                c.choices[i].selected = true;
                c.choices[i].enabled = true;
            }
            created.push(i);
        }

        c.sorted_out.clear();
        for &i in &created {
            if c.choices[i].optional && c.choices[i].selected {
                c.sort_out(i, false);
            }
        }
        Ok(c)
    }

    /// Index of the mod with this name (case does not matter).
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.choices.iter().position(|c| c.pack_mod.name.eq_ignore_ascii_case(name))
    }

    /// The indexes in the order the Java window lists them (by name).
    pub fn alphabetical(&self) -> Vec<usize> {
        let mut order: Vec<usize> = (0..self.choices.len()).collect();
        order.sort_by(|a, b| {
            self.choices[*a].pack_mod.name.to_lowercase().cmp(&self.choices[*b].pack_mod.name.to_lowercase())
        });
        order
    }

    pub fn has_optional(&self) -> bool {
        self.choices.iter().any(|c| c.optional)
    }

    pub fn has_recommended(&self) -> bool {
        self.choices.iter().any(|c| c.pack_mod.recommended)
    }

    /// The warning to ask about before this mod is switched on, if the pack wrote one for it.
    pub fn warning_for(&self, idx: usize, warnings: &HashMap<String, String>) -> Option<String> {
        let key = self.choices[idx].pack_mod.warning.as_ref()?;
        warnings.get(key).cloned()
    }

    fn linked_to(&self, idx: usize) -> Vec<usize> {
        let name = &self.choices[idx].pack_mod.name;
        (0..self.choices.len())
            .filter(|&j| {
                self.choices[j]
                    .pack_mod
                    .linked
                    .as_deref()
                    .is_some_and(|l| !l.is_empty() && l.eq_ignore_ascii_case(name))
            })
            .collect()
    }

    fn in_same_group(&self, idx: usize) -> Vec<usize> {
        let g = self.choices[idx].pack_mod.group.clone().unwrap_or_default();
        (0..self.choices.len())
            .filter(|&j| {
                j != idx
                    && self.choices[j]
                        .pack_mod
                        .group
                        .as_deref()
                        .is_some_and(|x| !x.is_empty() && x.eq_ignore_ascii_case(&g))
            })
            .collect()
    }

    fn dependencies_of(&self, idx: usize) -> Vec<usize> {
        self.choices[idx].pack_mod.depends_list().iter().filter_map(|n| self.index_of(n)).collect()
    }

    fn dependants_of(&self, idx: usize) -> Vec<usize> {
        let name = &self.choices[idx].pack_mod.name;
        (0..self.choices.len())
            .filter(|&j| self.choices[j].pack_mod.depends_list().iter().any(|d| d.eq_ignore_ascii_case(name)))
            .collect()
    }

    /// Apply what switching a box on or off means for the others.
    fn sort_out(&mut self, idx: usize, first_go: bool) {
        if first_go {
            self.sorted_out.clear();
        }
        if self.choices[idx].selected {
            for j in self.linked_to(idx) {
                self.choices[j].enabled = true;
            }
            if self.choices[idx].pack_mod.has_group() {
                for j in self.in_same_group(idx) {
                    self.choices[j].selected = false;
                }
            }
            if !self.choices[idx].pack_mod.depends_list().is_empty() {
                for j in self.dependencies_of(idx) {
                    if !self.sorted_out.contains(&j) {
                        self.sorted_out.push(j);
                        self.choices[j].selected = true;
                        self.sort_out(j, false);
                    }
                }
            }
        } else {
            for j in self.linked_to(idx) {
                self.choices[j].enabled = false;
                self.choices[j].selected = false;
            }
            let dependants = self.dependants_of(idx);
            if !dependants.is_empty() {
                for j in dependants {
                    self.choices[j].selected = false;
                }
            } else if !self.choices[idx].pack_mod.depends_list().is_empty() {
                // a library only wanted by this mod goes with it
                for j in self.dependencies_of(idx) {
                    if self.choices[j].pack_mod.library {
                        self.choices[j].selected = false;
                    }
                }
            }
        }
    }

    /// The player ticked or cleared a box. Boxes that are greyed out do nothing.
    pub fn toggle(&mut self, idx: usize, on: bool) {
        if idx >= self.choices.len() || !self.choices[idx].enabled {
            return;
        }
        self.choices[idx].selected = on;
        self.sort_out(idx, true);
    }

    /// "Select Recommended" (or "Select All" when nothing is marked recommended).
    pub fn select_recommended(&mut self) {
        let has_recommended = self.has_recommended();
        for i in 0..self.choices.len() {
            if !self.choices[i].optional {
                continue;
            }
            if self.choices[i].pack_mod.recommended {
                if self.choices[i].pack_mod.has_group() {
                    if self.only_recommended_in_group(i) {
                        self.choices[i].selected = true;
                        self.choices[i].enabled = true;
                        self.sort_out(i, false);
                    } else if has_recommended {
                        self.choices[i].selected = false;
                    }
                } else {
                    self.choices[i].selected = true;
                    self.choices[i].enabled = true;
                    self.sort_out(i, false);
                }
            } else {
                self.choices[i].selected = false;
            }
        }
    }

    fn only_recommended_in_group(&self, idx: usize) -> bool {
        let g = self.choices[idx].pack_mod.group.clone().unwrap_or_default();
        !self.choices.iter().enumerate().any(|(j, c)| {
            j != idx
                && c.pack_mod.group.as_deref().is_some_and(|x| !x.is_empty() && x.eq_ignore_ascii_case(&g))
                && c.pack_mod.recommended
        })
    }

    /// "Clear All".
    pub fn clear_all(&mut self) {
        for i in 0..self.choices.len() {
            if self.choices[i].optional {
                self.choices[i].selected = false;
                for j in self.linked_to(i) {
                    self.choices[j].enabled = false;
                }
            }
        }
    }

    /// Whether the window would offer anything at all.
    pub fn needs_asking(&self) -> bool {
        self.has_optional()
    }

    /// Every mod to install. With nothing optional that is every mod in install order; otherwise the ticked boxes,
    /// listed by name as the Java window does.
    pub fn selected(&self) -> Vec<PackMod> {
        if !self.has_optional() {
            return self.choices.iter().map(|c| c.pack_mod.clone()).collect();
        }
        self.alphabetical()
            .into_iter()
            .filter(|&i| self.choices[i].selected)
            .map(|i| self.choices[i].pack_mod.clone())
            .collect()
    }

    pub fn unselected(&self) -> Vec<PackMod> {
        self.alphabetical()
            .into_iter()
            .filter(|&i| !self.choices[i].selected)
            .map(|i| self.choices[i].pack_mod.clone())
            .collect()
    }

    /// Choose from a list of mod names (case-insensitive), then let linked and grouped mods settle. Used when a
    /// caller already knows what it wants (for example "keep what I picked last time").
    pub fn select_only(&mut self, names: &[String]) {
        for i in 0..self.choices.len() {
            if self.choices[i].optional {
                self.choices[i].selected = names.iter().any(|n| n.eq_ignore_ascii_case(&self.choices[i].pack_mod.name));
                self.choices[i].enabled = true;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(name: &str) -> PackMod {
        PackMod { name: name.into(), file: format!("{name}.jar"), ..Default::default() }
    }

    fn opt(name: &str) -> PackMod {
        PackMod { optional: true, ..m(name) }
    }

    #[test]
    fn required_mods_are_on_and_locked_optional_ones_start_off() {
        let c = ModChoices::new(vec![m("Core"), opt("Minimap")], false, None).unwrap();
        let core = &c.choices[c.index_of("Core").unwrap()];
        assert!(core.selected && !core.enabled);
        let map = &c.choices[c.index_of("minimap").unwrap()];
        assert!(!map.selected && map.enabled);
        let names: Vec<String> = c.selected().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["Core"]);
    }

    #[test]
    fn without_optional_mods_everything_installs_in_order() {
        let c = ModChoices::new(vec![m("B"), m("A")], false, None).unwrap();
        let names: Vec<String> = c.selected().into_iter().map(|m| m.name).collect();
        assert_eq!(names, vec!["B", "A"]);
        assert!(!c.needs_asking());
    }

    #[test]
    fn choosing_one_of_a_group_switches_the_others_off() {
        let mut a = opt("ShadersA");
        a.group = Some("shaders".into());
        let mut b = opt("ShadersB");
        b.group = Some("shaders".into());
        let mut c = ModChoices::new(vec![a, b], false, None).unwrap();
        let (ia, ib) = (c.index_of("ShadersA").unwrap(), c.index_of("ShadersB").unwrap());
        c.toggle(ia, true);
        assert!(c.choices[ia].selected && !c.choices[ib].selected);
        c.toggle(ib, true);
        assert!(c.choices[ib].selected && !c.choices[ia].selected);
    }

    #[test]
    fn dependencies_follow_and_dependants_go_with_them() {
        let lib = opt("LibMod");
        let mut addon = opt("Addon");
        addon.depends = Some(vec!["LibMod".into()]);
        let mut c = ModChoices::new(vec![lib, addon], false, None).unwrap();
        let (il, ia) = (c.index_of("LibMod").unwrap(), c.index_of("Addon").unwrap());
        c.toggle(ia, true);
        assert!(c.choices[il].selected, "turning on Addon turns on what it needs");
        c.toggle(il, false);
        assert!(!c.choices[ia].selected, "turning off LibMod turns off what needs it");
    }

    #[test]
    fn linked_mods_follow_their_parent() {
        let parent = opt("Parent");
        let mut child = opt("Child");
        child.linked = Some("Parent".into());
        let mut c = ModChoices::new(vec![parent, child], false, None).unwrap();
        let (ip, ic) = (c.index_of("Parent").unwrap(), c.index_of("Child").unwrap());
        assert!(!c.choices[ic].enabled, "a linked mod waits for its parent");
        c.toggle(ip, true);
        assert!(c.choices[ic].enabled);
        c.toggle(ic, true);
        assert!(c.choices[ic].selected);
        c.toggle(ip, false);
        assert!(!c.choices[ic].selected && !c.choices[ic].enabled);
        // install order puts the child right behind the parent
        let order: Vec<String> = sort_mods(&[opt("Parent"), opt("Other"), {
            let mut x = opt("Child");
            x.linked = Some("Parent".into());
            x
        }])
        .into_iter()
        .map(|m| m.name)
        .collect();
        assert_eq!(order, vec!["Parent", "Child", "Other"]);
    }

    #[test]
    fn a_linked_mod_to_nowhere_is_an_error() {
        let mut child = opt("Child");
        child.linked = Some("Ghost".into());
        assert!(ModChoices::new(vec![child], false, None).is_err());
    }

    #[test]
    fn a_reinstall_remembers_what_was_picked() {
        let mut was = crate::modrecord::ModRecord::new("Minimap", "m.jar", "mods");
        was.was_selected = false;
        let mut kept = crate::modrecord::ModRecord::new("Extras", "e.jar", "mods");
        kept.was_selected = true;
        let remembered = Remembered::from_records(&[was, kept]);
        let mut selected_by_default = opt("Minimap");
        selected_by_default.selected = true;
        let c = ModChoices::new(vec![selected_by_default, opt("Extras"), opt("Brand New")], false, Some(&remembered))
            .unwrap();
        assert!(!c.choices[c.index_of("Minimap").unwrap()].selected);
        assert!(c.choices[c.index_of("Extras").unwrap()].selected);
        assert!(!c.choices[c.index_of("Brand New").unwrap()].selected, "a mod the old install never saw starts off");
    }

    #[test]
    fn select_recommended_and_clear_all() {
        let mut rec = opt("Rec");
        rec.recommended = true;
        let mut not = opt("NotRec");
        not.recommended = false;
        let mut c = ModChoices::new(vec![rec, not], false, None).unwrap();
        c.select_recommended();
        assert!(c.choices[c.index_of("Rec").unwrap()].selected);
        assert!(!c.choices[c.index_of("NotRec").unwrap()].selected);
        c.clear_all();
        assert!(c.choices.iter().all(|x| !x.selected));
    }
}
