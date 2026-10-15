//! Works out what has to be installed alongside a mod, and whether anything stops it, before a single file is
//! downloaded. The result is a plan that can be shown to the player, with the reason for every line.

use std::collections::{HashMap, HashSet, VecDeque};

use crate::mods::api::MrVersion;

/// How one mod version relates to another project.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Relation {
    Required,
    Optional,
    Incompatible,
    Embedded,
}

/// A link from one mod version to another project.
#[derive(Clone, Debug)]
pub struct Dependency {
    pub project_id: String,
    pub relation: Relation,
    /// A specific version that is asked for, or `None` for any version.
    pub version_id: Option<String>,
}

/// One downloadable version of a mod, from any provider, described by what matters for dependency planning.
#[derive(Clone, Debug)]
pub struct ModVersion {
    pub project_id: String,
    pub version_id: String,
    pub name: String,
    /// Loaders it works with, empty meaning any.
    pub loaders: HashSet<String>,
    /// Minecraft versions it works with, empty meaning any.
    pub game_versions: HashSet<String>,
    pub dependencies: Vec<Dependency>,
}

impl ModVersion {
    pub fn new(project_id: &str, version_id: &str, name: &str) -> ModVersion {
        ModVersion {
            project_id: project_id.into(),
            version_id: version_id.into(),
            name: name.into(),
            loaders: HashSet::new(),
            game_versions: HashSet::new(),
            dependencies: Vec::new(),
        }
    }

    pub fn with_loaders(mut self, values: &[&str]) -> Self {
        self.loaders.extend(values.iter().map(|s| s.to_string()));
        self
    }

    pub fn with_game_versions(mut self, values: &[&str]) -> Self {
        self.game_versions.extend(values.iter().map(|s| s.to_string()));
        self
    }

    pub fn depends(mut self, project_id: &str, relation: Relation) -> Self {
        self.dependencies.push(Dependency { project_id: project_id.into(), relation, version_id: None });
        self
    }

    pub fn works_with(&self, loader: Option<&str>, game_version: Option<&str>) -> bool {
        let loader_ok = self.loaders.is_empty() || loader.is_none_or(|l| self.loaders.contains(l));
        let version_ok = self.game_versions.is_empty() || game_version.is_none_or(|g| self.game_versions.contains(g));
        loader_ok && version_ok
    }

    /// Turns Modrinth's description of a version into the form the planner works with.
    pub fn from_modrinth(version: &MrVersion) -> ModVersion {
        let name = if version.name.is_empty() {
            format!("{} {}", version.project_id, version.version_number)
        } else {
            version.name.clone()
        };
        let mut result = ModVersion::new(&version.project_id, &version.id, &name);
        result.loaders.extend(version.loaders.iter().cloned());
        result.game_versions.extend(version.game_versions.iter().cloned());
        for d in &version.dependencies {
            // a dependency without a project (a bare file) can't be planned
            let Some(project_id) = d.project_id.clone() else { continue };
            let relation = match d.dependency_type.as_str() {
                "required" => Relation::Required,
                "optional" => Relation::Optional,
                "incompatible" => Relation::Incompatible,
                "embedded" => Relation::Embedded,
                _ => continue,
            };
            result.dependencies.push(Dependency { project_id, relation, version_id: d.version_id.clone() });
        }
        result
    }
}

/// Looks up the versions of a project, newest first. Each provider supplies its own.
pub trait Catalog {
    fn versions_of(&self, project_id: &str) -> Vec<ModVersion>;
}

impl<F: Fn(&str) -> Vec<ModVersion>> Catalog for F {
    fn versions_of(&self, project_id: &str) -> Vec<ModVersion> {
        self(project_id)
    }
}

#[derive(Clone, Debug)]
pub struct Request {
    pub wanted: ModVersion,
    pub loader: Option<String>,
    pub game_version: Option<String>,
    /// Projects already in the instance (project id to the version installed).
    pub installed: HashMap<String, String>,
    /// Versions the player insists on (project id to version id).
    pub pinned: HashMap<String, String>,
}

impl Request {
    pub fn new(wanted: ModVersion) -> Request {
        Request { wanted, loader: None, game_version: None, installed: HashMap::new(), pinned: HashMap::new() }
    }
}

/// One mod that will be added, and why.
#[derive(Clone, Debug)]
pub struct Step {
    pub version: ModVersion,
    pub required_by: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    /// In install order: dependencies come before what needs them.
    pub adds: Vec<Step>,
    /// Things that stop the install, with the reason.
    pub conflicts: Vec<String>,
    /// Needs that can't be met, with the reason.
    pub problems: Vec<String>,
    /// Extras that go well with this mod but aren't needed.
    pub suggestions: Vec<String>,
    /// Needs that are already met by what is installed.
    pub already_installed: Vec<String>,
}

impl Plan {
    /// True if it is safe to go ahead.
    pub fn ok(&self) -> bool {
        self.conflicts.is_empty() && self.problems.is_empty()
    }
}

/// Mods in the order they were first met, with a lookup by project.
#[derive(Default)]
struct Planned {
    order: Vec<String>,
    steps: HashMap<String, Step>,
}

pub fn plan(request: &Request, catalog: &dyn Catalog) -> Plan {
    let mut plan = Plan::default();
    let mut planned = Planned::default();
    let mut visiting: HashSet<String> = HashSet::new();

    add(request, catalog, &mut plan, &mut planned, &mut visiting, &request.wanted, None);

    // anything that is incompatible with something that's installed or planned
    for id in &planned.order {
        let step = &planned.steps[id];
        for dep in &step.version.dependencies {
            if dep.relation == Relation::Incompatible
                && (request.installed.contains_key(&dep.project_id) || planned.steps.contains_key(&dep.project_id))
            {
                plan.conflicts.push(format!(
                    "{} doesn't work together with {}{}",
                    step.version.name,
                    dep.project_id,
                    if request.installed.contains_key(&dep.project_id) {
                        ", which you have installed"
                    } else {
                        ", which would also be installed"
                    }
                ));
            }
        }
    }

    // the other way round: something installed that rules out what's being added
    for id in &planned.order {
        let step = &planned.steps[id];
        for (installed_id, installed_version) in &request.installed {
            for installed in catalog.versions_of(installed_id) {
                if &installed.version_id != installed_version {
                    continue;
                }
                for dep in &installed.dependencies {
                    if dep.relation == Relation::Incompatible && dep.project_id == step.version.project_id {
                        let message =
                            format!("{} (installed) doesn't work together with {}", installed.name, step.version.name);
                        if !plan.conflicts.contains(&message) {
                            plan.conflicts.push(message);
                        }
                    }
                }
            }
        }
    }

    // dependencies first
    plan.adds = planned.order.iter().rev().map(|id| planned.steps[id].clone()).collect();
    plan
}

fn add(
    request: &Request,
    catalog: &dyn Catalog,
    plan: &mut Plan,
    planned: &mut Planned,
    visiting: &mut HashSet<String>,
    version: &ModVersion,
    required_by: Option<&str>,
) {
    if let Some(existing) = planned.steps.get_mut(&version.project_id) {
        if let Some(r) = required_by {
            if !existing.required_by.iter().any(|x| x == r) {
                existing.required_by.push(r.to_string());
            }
        }
        return;
    }
    if !visiting.insert(version.project_id.clone()) {
        return;
    }
    if !version.works_with(request.loader.as_deref(), request.game_version.as_deref()) {
        plan.problems.push(format!("{} isn't made for {}", version.name, describe(request)));
    }
    let mut step = Step { version: version.clone(), required_by: Vec::new() };
    if let Some(r) = required_by {
        step.required_by.push(r.to_string());
    }
    planned.order.push(version.project_id.clone());
    planned.steps.insert(version.project_id.clone(), step);

    let mut queue: VecDeque<&Dependency> = version.dependencies.iter().collect();
    while let Some(dep) = queue.pop_front() {
        match dep.relation {
            Relation::Optional => plan.suggestions.push(format!("{} works well with {}", version.name, dep.project_id)),
            Relation::Required => resolve(request, catalog, plan, planned, visiting, version, dep),
            _ => {}
        }
    }
}

fn resolve(
    request: &Request,
    catalog: &dyn Catalog,
    plan: &mut Plan,
    planned: &mut Planned,
    visiting: &mut HashSet<String>,
    parent: &ModVersion,
    dep: &Dependency,
) {
    if let Some(installed_version) = request.installed.get(&dep.project_id) {
        let pin = dep.version_id.as_ref().or_else(|| request.pinned.get(&dep.project_id));
        match pin {
            Some(pin) if pin != installed_version => plan.conflicts.push(format!(
                "{} needs version {} of {} but {} is installed",
                parent.name, pin, dep.project_id, installed_version
            )),
            _ => plan.already_installed.push(format!("{} (needed by {})", dep.project_id, parent.name)),
        }
        return;
    }

    let versions = catalog.versions_of(&dep.project_id);
    let wanted = dep.version_id.as_ref().or_else(|| request.pinned.get(&dep.project_id));

    let chosen = versions
        .iter()
        .filter(|c| wanted.is_none_or(|w| *w == c.version_id))
        .find(|c| c.works_with(request.loader.as_deref(), request.game_version.as_deref()));

    match chosen {
        None => plan.problems.push(format!(
            "{} needs {}, but {}",
            parent.name,
            dep.project_id,
            if versions.is_empty() {
                "it couldn't be found".to_string()
            } else if let Some(w) = wanted {
                format!("version {w} isn't available for {}", describe(request))
            } else {
                format!("no version of it works with {}", describe(request))
            }
        )),
        Some(chosen) => {
            let chosen = chosen.clone();
            add(request, catalog, plan, planned, visiting, &chosen, Some(&parent.name));
        }
    }
}

fn describe(request: &Request) -> String {
    let mut s = String::new();
    if let Some(l) = &request.loader {
        s.push_str(l);
        s.push(' ');
    }
    match &request.game_version {
        Some(g) => s.push_str(&format!("Minecraft {g}")),
        None => s.push_str("this Minecraft version"),
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    struct Fixture {
        catalog: RefCell<HashMap<String, Vec<ModVersion>>>,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture { catalog: RefCell::new(HashMap::new()) }
        }

        fn mod_(&self, project: &str, version: &str, deps: &[(&str, Relation)]) -> ModVersion {
            let mut m = ModVersion::new(project, version, &format!("{project} {version}"))
                .with_loaders(&["fabric"])
                .with_game_versions(&["1.20.1"]);
            for (p, r) in deps {
                m = m.depends(p, *r);
            }
            self.catalog.borrow_mut().entry(project.to_string()).or_default().push(m.clone());
            m
        }

        fn plan(&self, wanted: ModVersion, installed: &[(&str, &str)]) -> Plan {
            let mut r = Request::new(wanted);
            r.loader = Some("fabric".into());
            r.game_version = Some("1.20.1".into());
            for (k, v) in installed {
                r.installed.insert(k.to_string(), v.to_string());
            }
            plan(&r, &|id: &str| self.catalog.borrow().get(id).cloned().unwrap_or_default())
        }
    }

    fn names(p: &Plan) -> Vec<&str> {
        p.adds.iter().map(|s| s.version.project_id.as_str()).collect()
    }

    #[test]
    fn adds_required_dependencies_first_and_explains_why() {
        let f = Fixture::new();
        f.mod_("lib", "2.0", &[]);
        f.mod_("api", "1.0", &[("lib", Relation::Required)]);
        let main = f.mod_("main", "3.0", &[("api", Relation::Required)]);
        let p = f.plan(main, &[]);
        assert!(p.ok());
        assert_eq!(names(&p), vec!["lib", "api", "main"]);
        assert_eq!(p.adds[0].required_by[0], "api 1.0");
    }

    #[test]
    fn skips_what_is_already_installed() {
        let f = Fixture::new();
        f.mod_("api", "1.0", &[]);
        let main = f.mod_("main", "3.0", &[("api", Relation::Required)]);
        let p = f.plan(main, &[("api", "0.9")]);
        assert_eq!(names(&p), vec!["main"]);
        assert_eq!(p.already_installed.len(), 1);
    }

    #[test]
    fn picks_the_newest_version_that_fits_the_loader_and_game() {
        let f = Fixture::new();
        f.catalog.borrow_mut().insert(
            "api".into(),
            vec![
                ModVersion::new("api", "3", "api 3").with_loaders(&["forge"]).with_game_versions(&["1.20.1"]),
                ModVersion::new("api", "2", "api 2").with_loaders(&["fabric"]).with_game_versions(&["1.21"]),
                ModVersion::new("api", "1", "api 1").with_loaders(&["fabric"]).with_game_versions(&["1.20.1"]),
            ],
        );
        let main = f.mod_("main", "1", &[("api", Relation::Required)]);
        let p = f.plan(main, &[]);
        assert!(p.ok());
        assert_eq!(p.adds[0].version.version_id, "1");
    }

    #[test]
    fn reports_a_dependency_that_cannot_be_met() {
        let f = Fixture::new();
        f.catalog
            .borrow_mut()
            .insert("api".into(), vec![ModVersion::new("api", "1", "api 1").with_loaders(&["forge"])]);
        let main = f.mod_("main", "1", &[("api", Relation::Required), ("ghost", Relation::Required)]);
        let p = f.plan(main, &[]);
        assert!(!p.ok());
        assert_eq!(p.problems.len(), 2);
        assert!(p.problems[0].contains("no version of it works with fabric Minecraft 1.20.1"));
        assert!(p.problems[1].contains("couldn't be found"));
    }

    #[test]
    fn reports_incompatible_mods_in_both_directions() {
        let f = Fixture::new();
        f.mod_("rival", "1", &[]);
        let main = f.mod_("main", "1", &[("rival", Relation::Incompatible)]);
        assert!(f.plan(main, &[("rival", "1")]).conflicts[0].contains("which you have installed"));

        let other = f.mod_("other", "1", &[]);
        f.mod_("blocker", "1", &[("other", Relation::Incompatible)]);
        let p = f.plan(other, &[("blocker", "1")]);
        assert!(!p.ok());
        assert!(p.conflicts[0].contains("blocker 1 (installed)"));
    }

    #[test]
    fn survives_cycles_and_only_adds_each_mod_once() {
        let f = Fixture::new();
        let a = f.mod_("a", "1", &[("b", Relation::Required)]);
        f.mod_("b", "1", &[("a", Relation::Required)]);
        let p = f.plan(a, &[]);
        assert_eq!(p.adds.len(), 2);
        assert!(p.ok());
    }

    #[test]
    fn optional_dependencies_are_only_suggestions() {
        let f = Fixture::new();
        f.mod_("extra", "1", &[]);
        let main = f.mod_("main", "1", &[("extra", Relation::Optional)]);
        let p = f.plan(main, &[]);
        assert_eq!(names(&p), vec!["main"]);
        assert_eq!(p.suggestions.len(), 1);
    }

    #[test]
    fn flags_a_mod_that_is_not_made_for_the_instance() {
        let f = Fixture::new();
        let wrong = ModVersion::new("old", "1", "old 1").with_loaders(&["forge"]).with_game_versions(&["1.12.2"]);
        let p = f.plan(wrong, &[]);
        assert!(!p.ok());
        assert!(p.problems[0].contains("isn't made for fabric Minecraft 1.20.1"));
    }

    #[test]
    fn a_version_the_dependency_insists_on_is_the_one_chosen_or_a_conflict() {
        let f = Fixture::new();
        f.mod_("sodium", "0.5", &[]);
        f.mod_("sodium", "0.6", &[]);
        let mut iris = f.mod_("iris", "1", &[]);
        iris.dependencies.push(Dependency {
            project_id: "sodium".into(),
            relation: Relation::Required,
            version_id: Some("0.5".into()),
        });
        let p = f.plan(iris.clone(), &[]);
        assert_eq!(p.adds[0].version.version_id, "0.5");
        let conflict = f.plan(iris, &[("sodium", "0.6")]);
        assert!(conflict.conflicts[0].contains("needs version 0.5 of sodium but 0.6 is installed"));
    }

    #[test]
    fn converts_modrinth_versions() {
        let v = MrVersion::from_value(serde_json::json!({
            "id": "v1", "project_id": "sodium", "name": "Sodium 0.5", "loaders": ["fabric"], "game_versions": ["1.20.1"],
            "dependencies": [
                {"project_id": "indium", "dependency_type": "required"},
                {"file_name": "bare.jar", "dependency_type": "required"},
                {"project_id": "rival", "dependency_type": "incompatible"},
                {"project_id": "inside", "dependency_type": "embedded"}
            ]
        }))
        .unwrap();
        let m = ModVersion::from_modrinth(&v);
        assert_eq!(m.project_id, "sodium");
        assert!(m.works_with(Some("fabric"), Some("1.20.1")));
        assert!(!m.works_with(Some("forge"), Some("1.20.1")));
        assert_eq!(m.dependencies.len(), 3);
        assert_eq!(m.dependencies[0].relation, Relation::Required);
        assert_eq!(m.dependencies[1].relation, Relation::Incompatible);
        assert_eq!(m.dependencies[2].relation, Relation::Embedded);
    }
}
